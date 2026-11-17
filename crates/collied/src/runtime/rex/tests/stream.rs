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
    let rex = Arc::new(Rex::new(fake.socket.clone()));
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
