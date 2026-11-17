use std::sync::Arc;

use super::super::fake::{Fake, NONCE};
use super::super::stream::tests::{FORMAT, ROWS, STREAM};
use super::*;

const FORMAT_CALL: &str = "com.superlogical.terminal.format";

/// The fixture's stream up to its program's exit.
fn running() -> Vec<u8> {
    let exit = format!("\x1b_rex1;{NONCE};{{\"type\":\"child_exited\"");
    let at = (STREAM.windows(exit.len()))
        .position(|w| w == exit.as_bytes())
        .unwrap();
    STREAM[..at].to_vec()
}

/// A followed fake whose agent in `B1` streams `stream`, and whose `format` answers what the
/// fixture's Rex answered, so a polled read reads as a streamed one.
async fn streaming(stream: Vec<u8>) -> (Fake, Arc<Rex>, Runtime, tempfile::TempDir) {
    streaming_marked(stream, None).await
}

async fn streaming_marked(
    stream: Vec<u8>,
    marker: Option<std::path::PathBuf>,
) -> (Fake, Arc<Rex>, Runtime, tempfile::TempDir) {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        let b = s.block(B1);
        b.screen = String::from_utf8(FORMAT.to_vec()).unwrap();
        b.rows = Some(String::from_utf8(ROWS.to_vec()).unwrap());
        b.size = (100, 30);
        s.data.stream = stream;
    }
    let mut rex = Rex::new(fake.socket.clone());
    if let Some(marker) = marker {
        rex = rex.with_stream_marker(marker);
    }
    let rex = Arc::new(rex);
    let _task = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    (fake, rex, rt, dir)
}

fn live(rex: &Rex) -> bool {
    rex.streams.read(&id(B1), "visible", None).is_some()
}

fn formats(fake: &Fake, from: usize) -> usize {
    fake.calls()[from..]
        .iter()
        .filter(|m| *m == FORMAT_CALL)
        .count()
}

async fn recent(rt: &Runtime) -> String {
    rt.agent_read(&id(B1), "recent_unwrapped", Some(200))
        .await
        .unwrap()
        .text
}

/// Logs at debug level from here on, for the stream's decisions.
fn logging() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (logs, tracing::subscriber::set_default(subscriber))
}

fn logged(logs: &Logs, line: &str) -> bool {
    String::from_utf8_lossy(&crate::lock(&logs.0)).contains(line)
}

#[tokio::test]
async fn watches_share_one_stream_read_without_format_and_closed_with_the_last() {
    let (fake, rex, rt, _dir) = streaming(running()).await;
    let polled = recent(&rt).await;
    let detection = rt.detection_text(&id(B1)).await.unwrap();
    let first = rt.watch(&id(B1)).unwrap();
    let second = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    let from = fake.calls().len();
    assert_eq!(recent(&rt).await, polled);
    assert_eq!(rt.detection_text(&id(B1)).await.unwrap(), detection);
    assert_eq!(formats(&fake, from), 0);
    fake.data_send(b"one more line\r\n");
    eventually("the new line", || {
        rex.streams
            .read(&id(B1), "visible", None)
            .is_some_and(|t| t.contains("one more line"))
    })
    .await;
    assert_eq!(formats(&fake, from), 0);
    assert_eq!(fake.state().data.handshakes, [(1, 0)]);

    drop(first);
    assert!(live(&rex));
    drop(second);
    eventually("the stream closed", || !fake.state().data.closed.is_empty()).await;
    // Status 1000 ends Rex's stream; any other close keeps it waiting for a resume.
    assert_eq!(fake.state().data.closed, [Some(1000)]);
    let from = fake.calls().len();
    recent(&rt).await;
    assert_eq!(formats(&fake, from), 1);
}

#[tokio::test]
async fn a_dropped_stream_resumes_with_its_latest_token_and_every_byte_received() {
    let stream = running();
    let (fake, rex, rt, _dir) = streaming(stream.clone()).await;
    let _watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    fake.data_drop();
    fake.data_send(b"sent while away\r\n");
    eventually("the resumed stream", || {
        rex.streams
            .read(&id(B1), "visible", None)
            .is_some_and(|t| t.contains("sent while away"))
    })
    .await;
    // The fake welcomes a RESUME only with the latest token and the exact count.
    assert_eq!(
        fake.state().data.handshakes,
        [(1, 0), (2, stream.len() as u64)]
    );
    fake.data_drop();
    fake.data_send(b"and again\r\n");
    eventually("the second resume", || {
        rex.streams
            .read(&id(B1), "visible", None)
            .is_some_and(|t| t.contains("and again"))
    })
    .await;
    let resumed = stream.len() + "sent while away\r\n".len();
    assert_eq!(fake.state().data.handshakes[2], (2, resumed as u64));
}

#[tokio::test]
async fn a_refused_handshake_or_an_unknown_snapshot_version_polls_format() {
    let mut newer = running();
    newer[8] = 2;
    for (stream, reject, why) in [
        (
            running(),
            Some(0x01),
            "refused the data connection (reason 1)",
        ),
        (newer, None, "snapshot version 2, where this build reads 1"),
    ] {
        let (logs, _logging) = logging();
        let (fake, rex, rt, _dir) = streaming(stream).await;
        fake.state().data.reject = reject;
        let polled = recent(&rt).await;
        let _watch = rt.watch(&id(B1)).unwrap();
        eventually(why, || logged(&logs, why)).await;
        assert!(!live(&rex));
        let from = fake.calls().len();
        assert_eq!(recent(&rt).await, polled);
        assert_eq!(formats(&fake, from), 1);
        assert_eq!(fake.state().data.handshakes, [(1, 0)]);
    }
}

#[tokio::test]
async fn a_stream_whose_program_exited_is_not_resumed() {
    let (logs, _logging) = logging();
    let (fake, rex, rt, _dir) = streaming(STREAM.to_vec()).await;
    let _watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    fake.data_drop();
    eventually("the stream over", || logged(&logs, "Rex stream over")).await;
    assert!(!live(&rex));
    assert_eq!(fake.state().data.handshakes, [(1, 0)]);
}

/// The first `n` handshakes, waited for on the paused clock, which runs ahead while nothing
/// else can; the fake's notice, not a timer, ends the wait, so a client that loops fails here.
async fn handshakes(fake: &Fake, n: usize) -> Vec<(u8, u64)> {
    let shaken = fake.state().data.shaken.clone();
    loop {
        let handshakes = fake.state().data.handshakes.clone();
        if handshakes.len() >= n {
            return handshakes[..n].to_vec();
        }
        let wait = std::time::Duration::from_secs(300);
        let notified = tokio::time::timeout(wait, shaken.notified()).await;
        assert!(notified.is_ok(), "{handshakes:?}");
    }
}

fn gap(fake: &Fake, i: usize) -> std::time::Duration {
    let at = &fake.state().data.at;
    at[i] - at[i - 1]
}

#[tokio::test(start_paused = true)]
async fn a_refused_resume_starts_a_new_stream_after_the_delay() {
    let stream = running();
    let (fake, rex, rt, _dir) = streaming(stream.clone()).await;
    fake.state().data.refuse_resume = true;
    let _watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    fake.data_drop();
    let seen = handshakes(&fake, 3).await;
    assert_eq!(seen, [(1, 0), (2, stream.len() as u64), (1, 0)]);
    assert!(gap(&fake, 2) >= RETRY_FIRST, "{:?}", gap(&fake, 2));
    eventually("the new stream restored", || live(&rex)).await;
}

#[tokio::test(start_paused = true)]
async fn a_resumed_connection_that_ends_before_any_byte_starts_a_new_stream() {
    let stream = running();
    let (fake, rex, rt, _dir) = streaming(stream.clone()).await;
    fake.state().data.drop_resumed = true;
    let _watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    fake.data_drop();
    let seen = handshakes(&fake, 3).await;
    assert_eq!(seen, [(1, 0), (2, stream.len() as u64), (1, 0)]);
    assert!(gap(&fake, 2) >= RETRY_FIRST, "{:?}", gap(&fake, 2));
}

/// Rex accepts each stream and soon ends it with 1013: each new one waits twice as long.
#[tokio::test(start_paused = true)]
async fn streams_ended_soon_after_they_start_are_asked_for_less_and_less() {
    let (fake, rex, rt, _dir) = streaming(running()).await;
    let _watch = rt.watch(&id(B1)).unwrap();
    for n in 1..=5 {
        handshakes(&fake, n).await;
        eventually("the stream restored", || live(&rex)).await;
        fake.data_close(1013);
    }
    handshakes(&fake, 6).await;
    let mut wait = RETRY_FIRST;
    for i in 1..6 {
        assert!(gap(&fake, i) >= wait, "{i}: {:?} < {wait:?}", gap(&fake, i));
        wait = (wait * 2).min(RETRY_MAX);
    }
}

#[tokio::test]
async fn a_marker_left_by_an_earlier_run_turns_streaming_off() {
    let marked = tempfile::tempdir().unwrap();
    let marker = marked.path().join("rex-streaming");
    std::fs::write(&marker, "").unwrap();
    let (fake, _rex, rt, _dir) = streaming_marked(running(), Some(marker)).await;
    assert!(rt.watch(&id(B1)).unwrap().is_none());
    let from = fake.calls().len();
    recent(&rt).await;
    assert_eq!(formats(&fake, from), 1);
    assert!(!fake.calls().iter().any(|m| m == "data.connect"));
}

#[tokio::test]
async fn the_marker_lasts_while_a_stream_does() {
    let marked = tempfile::tempdir().unwrap();
    let marker = marked.path().join("rex-streaming");
    let (_fake, rex, rt, _dir) = streaming_marked(running(), Some(marker.clone())).await;
    let watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    assert!(marker.exists());
    drop(watch);
    eventually("the marker removed", || !marker.exists()).await;
}

/// A screen of alternating 1,500-byte links formats past what one `format` answer holds.
#[tokio::test]
async fn a_screen_that_formats_too_large_is_polled() {
    let (fake, rex, rt, _dir) = streaming(running()).await;
    let _watch = rt.watch(&id(B1)).unwrap();
    eventually("the stream restored", || live(&rex)).await;
    let target = format!("https://example.com/{}", "x".repeat(1500));
    let links: String = (0..100 * 30)
        .map(|i| format!("\x1b]8;;{target}{}\x1b\\{}", i % 2, i % 10))
        .collect();
    fake.data_send(links.as_bytes());
    eventually("the stream to stop answering", || !live(&rex)).await;
    let from = fake.calls().len();
    recent(&rt).await;
    assert_eq!(formats(&fake, from), 1);
}
