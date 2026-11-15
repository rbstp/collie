use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::json;

use super::client::Method;
use super::fake::{self, Fake};
use super::tests::{B1, B2, Logs, S1, T0, claude, eventually, follow, id};
use super::*;
use crate::runtime::Runtime;

const VT: &str =
    include_str!("../../../tests/fixtures/rex-0.1.0-preview.19/claude-2.1.325-permission.vt");
const TEXT: &str =
    include_str!("../../../tests/fixtures/rex-0.1.0-preview.19/claude-2.1.325-permission.txt");

/// A followed fake whose agent in `B1` shows `screen` at `size`, and the runtime routing to it.
async fn showing(screen: String, size: (u32, u32)) -> (Fake, Arc<Rex>, Runtime, tempfile::TempDir) {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    {
        let mut s = fake.state();
        let b = s.block(B1);
        b.screen = screen;
        b.size = size;
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    (fake, rex, rt, dir)
}

fn plain(rows: &str) -> Vec<String> {
    let mut lines: Vec<String> = rows.lines().map(|l| l.trim_end().to_owned()).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn strip(ansi: &str) -> String {
    let mut text = String::new();
    let mut rest = ansi;
    while let Some(at) = rest.find('\u{1b}') {
        text.push_str(&rest[..at]);
        rest = rest[at..].split_once('m').map_or("", |(_, after)| after);
    }
    text.push_str(rest);
    text
}

fn reads(fake: &Fake, from: usize) -> Vec<String> {
    fake.calls()[from..].to_vec()
}

const FORMAT: &str = "com.superlogical.terminal.format";
const BLOB_READ: &str = "com.superlogical.terminal.blob_read";

/// Rex's `format` VT of Claude Code's permission dialog (198x68, from Rex.app), below more
/// scrollback than the first tail holds, emulated from the tail: the screen reads as Rex's
/// own `format` text of it.
#[tokio::test]
async fn the_emulated_screen_reads_as_rex_formats_its_text() {
    let history: String = (1..=20_000).map(|i| format!("earlier {i}\r\n")).collect();
    let (fake, _rex, rt, _dir) = showing(format!("{history}{VT}"), (198, 68)).await;
    let want = plain(TEXT);
    assert!(want.iter().any(|l| l.contains("Do you want to proceed?")));
    let from = fake.calls().len();
    assert_eq!(plain(&rt.detection_text(&id(B1)).await.unwrap()), want);
    let visible = rt.agent_read(&id(B1), "visible", None).await.unwrap().text;
    assert!(visible.contains("\u{1b}[38;2;153;153;153m"), "{visible:?}");
    assert_eq!(plain(&strip(&visible)), want);
    assert_eq!(rt.pane_columns(&id(B1)).await.unwrap(), Some(198));
    let recent = rt
        .pane_read(&id(B1), "recent_unwrapped", Some(70))
        .await
        .unwrap();
    let recent = plain(&strip(&recent.text));
    let (history, screen) = recent.split_at(recent.len() - want.len());
    assert_eq!(screen, want);
    assert_eq!(history.last().map(String::as_str), Some("earlier 20000"));
    assert!(reads(&fake, from).iter().any(|m| m == BLOB_READ));
}

fn numbered(n: usize) -> String {
    (1..=n)
        .map(|i| format!("\u{1b}[0m\u{1b}[38;5;2mline {i}\u{1b}[0m of the scrollback"))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// A terminal longer than one answer is read as `format` and its tail; one that fit last
/// time, by `format` alone. Either way an unchanged tail at an unchanged size is not emulated.
#[tokio::test]
async fn an_unchanged_screen_is_never_emulated_again() {
    let (fake, rex, rt, _dir) = showing(numbered(5000), (80, 24)).await;
    let emulated = || rex.screens.emulated.load(Ordering::Relaxed);
    let read = async || {
        rt.agent_read(&id(B1), "recent_unwrapped", Some(30))
            .await
            .unwrap()
    };
    let first = read().await;
    let from = fake.calls().len();
    assert_eq!(read().await.text, first.text);
    assert_eq!(reads(&fake, from), [FORMAT, BLOB_READ]);
    assert_eq!(emulated(), 1);

    fake.state().block(B1).screen = numbered(5001);
    assert!(strip(&read().await.text).ends_with("line 5001 of the scrollback"));
    assert_eq!(emulated(), 2);
    fake.state().block(B1).size = (100, 24);
    fake.emit(|_| {
        let size = json!({ "columns": 100, "rows": 24, "cell_width_px": 8, "cell_height_px": 16, "owner": null });
        vec![fake::block_event(S1, B1, "size_changed", size)]
    });
    eventually("the new width", || {
        rex.columns(&id(B1)).ok() == Some(Some(100))
    })
    .await;
    read().await;
    assert_eq!(emulated(), 3);

    let visible = async |lines| rt.agent_read(&id(B1), "visible", lines).await.unwrap();
    visible(Some(5)).await;
    visible(None).await;
    assert_eq!(
        emulated(),
        4,
        "visible reads the same whatever lines it is given"
    );

    fake.state().block(B1).screen = numbered(100);
    read().await;
    let from = fake.calls().len();
    let short = read().await;
    assert_eq!(reads(&fake, from), [FORMAT]);
    assert!(strip(&short.text).ends_with("line 100 of the scrollback"));
    assert_eq!(emulated(), 5);
}

/// Readers of one block that ask while its `format` is under way share the next one: each
/// is served a rendering Rex made after it asked, never the one under way.
#[tokio::test]
async fn readers_that_ask_together_share_a_format_made_after_they_asked() {
    let (fake, _rex, rt, _dir) = showing(numbered(5000), (80, 24)).await;
    fake.state().hook(
        Method::Format,
        Some(B1),
        true,
        Box::new(|s| {
            s.block(B1).screen = numbered(5001);
            Vec::new()
        }),
    );
    let agent = id(B1);
    let from = fake.calls().len();
    let visible = async || rt.agent_read(&agent, "visible", None).await.unwrap().text;
    let (first, a, b, c) = tokio::join!(rt.detection_text(&agent), visible(), visible(), visible());
    assert!(first.unwrap().ends_with("line 5000 of the scrollback"));
    for later in [a, b, c] {
        assert!(strip(&later).ends_with("line 5001 of the scrollback"));
    }
    let formats = reads(&fake, from).iter().filter(|m| *m == FORMAT).count();
    assert_eq!(formats, 2);
}

/// Rex keeps a block's newest four results only: one dropped before its tail is read is
/// formatted again, and a read that never finds its result fails without ending anything.
#[tokio::test]
async fn a_result_rex_dropped_is_formatted_again() {
    let (fake, _rex, rt, _dir) = showing(numbered(5000), (80, 24)).await;
    let drop = || -> fake::Hook {
        Box::new(|s| {
            s.results.clear();
            Vec::new()
        })
    };
    fake.state().hook(Method::BlobRead, Some(B1), false, drop());
    let from = fake.calls().len();
    let read = rt.agent_read(&id(B1), "visible", None).await.unwrap();
    assert!(strip(&read.text).ends_with("line 5000 of the scrollback"));
    assert_eq!(reads(&fake, from), [FORMAT, BLOB_READ, FORMAT, BLOB_READ]);

    for _ in 0..3 {
        fake.state().hook(Method::BlobRead, Some(B1), false, drop());
    }
    assert!(rt.agent_read(&id(B1), "visible", None).await.is_err());
    assert!(rt.agent_read(&id(B1), "visible", None).await.is_ok());
}

/// 100,000 lines of scrollback: each read moves the tail its lines need, never the whole
/// rendering, and calls nothing but `format` and `blob_read`.
#[tokio::test]
async fn a_long_scrollback_moves_only_the_tail() {
    let (fake, _rex, rt, _dir) = showing(numbered(100_000), (80, 24)).await;
    let total = fake.state().block(B1).screen.len();
    assert!(total > 4_000_000);
    let from = fake.calls().len();
    fake.state().moved = 0;
    let read = rt
        .agent_read(&id(B1), "recent_unwrapped", Some(50))
        .await
        .unwrap();
    let rows = plain(&strip(&read.text));
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[49], "line 100000 of the scrollback");
    assert!(
        fake.state().moved <= (64 << 10) + 1,
        "{}",
        fake.state().moved
    );

    fake.state().moved = 0;
    let read = rt
        .agent_read(&id(B1), "recent_unwrapped", Some(1000))
        .await
        .unwrap();
    let rows = plain(&strip(&read.text));
    assert_eq!(rows[0], "line 99001 of the scrollback");
    assert!(fake.state().moved < total / 8, "{}", fake.state().moved);
    assert!(
        reads(&fake, from)
            .iter()
            .all(|m| m == FORMAT || m == BLOB_READ),
        "{:?}",
        reads(&fake, from)
    );
}

/// A tail whose escapes make it too long for one answer is read shorter, not forever.
#[tokio::test]
async fn a_tail_too_long_for_one_answer_is_halved() {
    let line = "\u{1b}[1m\u{1b}[0m".repeat(100);
    let screen: Vec<String> = (1..=3000).map(|i| format!("{line}{i}")).collect();
    let (_fake, _rex, rt, _dir) = showing(screen.join("\r\n"), (80, 24)).await;
    let read = rt
        .agent_read(&id(B1), "recent_unwrapped", Some(1000))
        .await
        .unwrap();
    let rows = plain(&strip(&read.text));
    assert_eq!(rows.last().map(String::as_str), Some("3000"));
    assert!(rows.len() < 1000);
}

/// A block whose reads fail is left out until its next report has it read again, and is
/// logged once however often it reports meanwhile.
#[tokio::test]
async fn a_block_whose_reads_failed_is_read_again_on_its_next_report() {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let skipped = || {
        String::from_utf8_lossy(&crate::lock(&logs.0))
            .matches("Rex block skipped")
            .count()
    };
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let undated = json!({ "state": "idle", "owner": { "pid": 20, "name": "2.1.322" } });
    let process = fake::process(20, "claude", "2.1.322");
    fake.state().add(S1, B2, process.clone(), vec![undated]);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    assert!(rex.get(&id(B2)).is_err());
    assert_eq!(skipped(), 1);

    for reads in [3, 4] {
        let fg = process["foreground"].clone();
        fake.emit(move |_| {
            vec![fake::block_event(
                S1,
                B2,
                "process_changed",
                json!({ "foreground": fg }),
            )]
        });
        eventually("the report read", || {
            fake.calls()
                .iter()
                .filter(|m| m.ends_with(".process"))
                .count()
                == reads
        })
        .await;
    }
    assert!(rex.get(&id(B2)).is_err());

    let record = fake::record("working", 20, "2.1.322", T0);
    fake.emit(move |s| {
        s.block(B2).records = vec![record.clone()];
        vec![fake::status_changed(S1, B2, record)]
    });
    eventually("the block read again", || rex.get(&id(B2)).is_ok()).await;
    assert_eq!(rex.get(&id(B2)).unwrap().agent_status, "working");
    assert_eq!(skipped(), 1);
}

/// One answer holds at most 983,040 bytes as a JSON string, its quotes included: once a
/// reader learns the longest tail that fits, a poll asks only for that.
#[tokio::test]
async fn a_poll_asks_only_for_a_tail_that_fits() {
    let line = "y".repeat(1000);
    let screen: Vec<String> = (1..=3000).map(|i| format!("{line}{i}")).collect();
    let (fake, _rex, rt, _dir) = showing(screen.join("\r\n"), (80, 24)).await;
    let read = async || {
        rt.agent_read(&id(B1), "recent_unwrapped", Some(1000))
            .await
            .unwrap()
    };
    let first = read().await;
    let from = fake.calls().len();
    fake.state().moved = 0;
    assert_eq!(read().await.text, first.text);
    assert_eq!(reads(&fake, from), [FORMAT, BLOB_READ]);
    assert!(fake.state().moved <= 600_000, "{}", fake.state().moved);
    assert!(strip(&first.text).ends_with("3000"));
}

/// A line longer than any tail, soft-wrapped across the whole screen: the screen reads
/// row by row, as Rex draws it, and a read that would join it fails rather than show a
/// screen without it.
#[tokio::test]
async fn a_line_longer_than_any_tail_never_shows_a_wrong_screen() {
    let short: String = (1..=50).map(|i| format!("short {i}\r\n")).collect();
    let long = "x".repeat(2_000_000);
    let (fake, _rex, rt, _dir) = showing(format!("{short}{long}\r\nprompt> "), (80, 24)).await;
    let rows: Vec<String> = (long.as_bytes().chunks(80))
        .map(|c| String::from_utf8(c.to_vec()).unwrap())
        .collect();
    fake.state().block(B1).rows = Some(format!("{short}{}\r\nprompt> ", rows.join("\r\n")));
    let visible = rt.agent_read(&id(B1), "visible", None).await.unwrap().text;
    let visible: Vec<&str> = visible.split("\r\n").collect();
    assert_eq!(visible.len(), 24);
    assert!(visible[..23].iter().all(|r| *r == "x".repeat(80)));
    assert_eq!(visible[23], "prompt>");
    assert!(
        rt.agent_read(&id(B1), "recent_unwrapped", Some(10))
            .await
            .is_err()
    );
}

/// A block is listed once `process` names an agent, so its size is read before: a read is
/// never emulated at no size, and `pane_columns` never answers 0.
#[tokio::test]
async fn a_blocks_size_is_read_before_it_is_listed() {
    let (fake, _rex, _rt, _dir) = showing(String::new(), (80, 24)).await;
    let calls = fake.calls();
    let at = |m: &str| calls.iter().position(|c| c == m).unwrap();
    assert!(at("com.superlogical.terminal.size") < at("com.superlogical.terminal.process"));
}
