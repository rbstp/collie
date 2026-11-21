use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde_json::Value;

use super::{Kind, TAIL_BYTES, Transcripts, history, open, tail, timestamp_ms};
use crate::prompt::{Question, Refused};
use crate::runtime::AgentInfo;

/// The transcript of a terminal's Claude Code session and what ties a call in it to that
/// terminal's process, so the file is read without the lock on [`Transcripts`].
pub struct Asking {
    path: PathBuf,
    session: String,
    started_at: u64,
}

/// What a session waits on, as its transcript says. "Not known" is kept apart from "nothing":
/// only a transcript read whole to the last message says no question is pending.
#[derive(Debug, PartialEq)]
pub enum Waits {
    /// One call, an AskUserQuestion: what an approval matches a question form against
    /// ([`crate::prompt::Menu::asks`]), with the session and the call's id, which the
    /// approval binds so that a later call asking the same question is another approval.
    Question {
        session: String,
        call: String,
        questions: Vec<Question>,
    },
    /// One Code Mode script that names AskUserQuestion, running or waiting to: its whole
    /// string literals, which a question form is matched against
    /// ([`crate::prompt::Menu::scripted`]), or none when the script was not read whole.
    Script {
        session: String,
        call: String,
        literals: Option<Vec<String>>,
    },
    /// One call that is no question.
    Other,
    /// No call and no script, in a session the read shows is not in Code Mode, and the turn is
    /// open: the transcript ends, records of Claude Code's own aside, at a prompt or at the
    /// results of the last calls, written by this session's process. Claude Code 2.1.329
    /// writes a message that holds an AskUserQuestion call only once the call has its result,
    /// so this is all the transcript holds while a question asked outside Code Mode is on
    /// screen. The version is that line's.
    Open {
        version: Option<[u32; 3]>,
    },
    /// No call, no script, and the turn is over or the session is in Code Mode.
    Nothing,
    Unknown(Refused),
}

impl Transcripts {
    /// The session is the terminal's by process ([`Transcripts::holder`]).
    pub fn asking(&self, a: &AgentInfo) -> Result<Asking, Refused> {
        if a.agent.as_deref() != Some("claude") {
            return Err(Refused::NoSession);
        }
        let (session, started_at) = self.holder(a)?;
        let path = self.locate(Kind::Claude, &session, a);
        Ok(Asking {
            path: path.ok_or(Refused::Transcript)?,
            session,
            started_at,
        })
    }
}

impl Asking {
    pub fn waits(&self) -> Waits {
        let read = open(&self.path).and_then(|(file, len, _)| Some((tail(file, len).ok()?, len)));
        match read {
            Some((text, len)) => pending(&text, len <= TAIL_BYTES, &self.session, self.started_at),
            None => Waits::Unknown(Refused::Transcript),
        }
    }
}

/// A tool call of the transcript: its block, and of its line the session, the time and how
/// the message ended.
struct Call {
    call: Value,
    by: Value,
    at: Option<u64>,
    stop: Value,
    version: Option<[u32; 3]>,
}

impl Call {
    fn of(call: &Value, line: &Value) -> Self {
        Self {
            call: call.clone(),
            by: line["sessionId"].clone(),
            at: line["timestamp"].as_str().and_then(timestamp_ms),
            stop: line["message"]["stop_reason"].clone(),
            version: line["version"].as_str().and_then(|v| {
                let mut parts = v.split('.').map(|p| p.parse().ok());
                Some([parts.next()??, parts.next()??, parts.next()??])
            }),
        }
    }

    /// A Code Mode script that names AskUserQuestion, and its text.
    fn asks(&self) -> Option<&str> {
        let code = self.call["input"]["code"].as_str()?;
        (self.call["name"] == "REPL" && code.contains("AskUserQuestion")).then_some(code)
    }
}

/// What the read says the session waits on, from two things read backward from its end.
///
/// The calls of the last assistant message that have no result. That message starts at an
/// assistant line of another message, or, when the lines carry no message id, at a `user`
/// line that is not only tool results. Claude Code writes one line per content block, so
/// parallel calls are several lines of one message, and a result or another line can come
/// between them. A `user` line that is no result under the last call ends the turn: none is
/// pending.
///
/// The scripts still running, over the whole read. In Code Mode (2.1.329) the model's one
/// tool is a script runner, `REPL`, and a question is a function its script calls: the
/// transcript holds nothing of the question but the script's text. Where Claude Code asks
/// before it runs a tool, that call has no result until its script ends. In auto mode it
/// answers the call at once with a receipt, `(dispatched #N)`, lets the script run, and
/// writes a `poll_events` attachment that names script N in a `repl-eval` event when it
/// ended. So a call with a receipt and no such event under it is still running; the numbers
/// start again in a new context, so an event counts for the nearest receipt above it. A
/// script whose line is older than the process (`since`) died with the process that ran it.
///
/// One script that names AskUserQuestion, running or without a result, is what a question
/// form is matched against, whatever else waits beside it: Claude Code's own wait for its
/// scripts (`Poll`), or another call, whose permission prompt is no question form. One
/// AskUserQuestion call with no result and nothing else pending is the question outside Code
/// Mode. With more than one of either, the screen does not say which form is drawn.
///
/// Nothing is concluded from a read that is not whole. A last line without its newline is
/// one Claude Code is still writing, and a line that does not parse may be cut: either can
/// be the result of the call above it, or a second call. `whole` says the text starts at the
/// file's start; otherwise a read that ends before the last message's start says nothing of
/// the calls above it, nor one that holds a result that reads as a receipt or as events and
/// not its call. A script that started
/// before the read's first line is not seen: its question is refused as no pending call. The
/// one call must be the session's and no older than its process, so one that a killed
/// session left in a transcript another process resumed is not this terminal's.
fn pending(text: &str, whole: bool, session: &str, since: u64) -> Waits {
    let unknown = Waits::Unknown(Refused::Transcript);
    if !text.is_empty() && !text.ends_with('\n') {
        return unknown;
    }
    let mut lines = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v["isSidechain"] == true => {}
            Ok(v) => lines.push(v),
            Err(_) => return unknown,
        }
    }
    fn blocks(v: &Value) -> &[Value] {
        let blocks = v["message"]["content"].as_array();
        blocks.map(Vec::as_slice).unwrap_or_default()
    }
    fn id(id: &Value) -> &str {
        id.as_str().unwrap_or_default()
    }
    // The tool each call names: a result is read as its own call's, since any tool's output
    // can hold the text of a receipt or of an event.
    let tools: HashMap<&str, &str> = (lines.iter().flat_map(blocks))
        .filter(|b| b["type"] == "tool_use")
        .map(|call| (id(&call["id"]), id(&call["name"])))
        .collect();
    // Whether the read shows a session in Code Mode, and whether it shows the session's mode
    // at all: Claude Code 2.1.329 writes `permission-mode` records through a session, each
    // followed in Code Mode by a `repl-mode` one. A read that starts inside the file and
    // holds neither says nothing of a script above it.
    let mut code_mode = tools.values().any(|tool| *tool == "REPL");
    let mut moded = whole;
    let mut results = HashSet::new();
    let mut made = HashSet::new();
    let mut calls = Vec::new();
    let mut message = None;
    let mut last = true;
    let mut bounded = whole;
    let mut ended = HashSet::new();
    let mut receipts = HashSet::new();
    let mut running = Vec::new();
    let mut end = None;
    for v in lines.iter().rev() {
        let blocks = blocks(v);
        match v["type"].as_str() {
            Some("repl-mode") if v["replMode"] != false => code_mode = true,
            Some("permission-mode") => moded = true,
            Some("attachment") if v["attachment"]["type"] == "poll_events" => {
                let envelopes = v["attachment"]["envelopes"].as_array();
                for envelope in envelopes.into_iter().flatten() {
                    match envelope.as_str().and_then(settled) {
                        Some(Some(n)) => _ = ended.insert(n),
                        // A script ended, and the event does not say which.
                        Some(None) => return unknown,
                        None => {}
                    }
                }
            }
            // Records Claude Code writes as the user's that are no prompt (history.rs reads
            // them as events): none says the model is at work on a turn.
            Some("user")
                if v["isMeta"] == true
                    || v["isCompactSummary"] == true
                    || v["origin"]["kind"] == "task-notification"
                    || history::local(v).is_some_and(|s| {
                        history::command(s).is_some() || s.starts_with("<local-command-stdout>")
                    }) => {}
            Some("user") => {
                let result = |b: &&Value| b["type"] == "tool_result";
                // Claude Code writes this line when a call is interrupted: the turn is over.
                let interrupted = matches!(blocks, [b] if b["text"].as_str().is_some_and(|t| t.starts_with("[Request interrupted")));
                end.get_or_insert_with(|| (!interrupted).then(|| Call::of(&Value::Null, v)));
                if last
                    && (blocks.is_empty() || !blocks.iter().all(|b| result(&b)))
                    && !matches!(message, Some(Value::String(_)))
                {
                    (bounded, last) = (true, false);
                }
                for block in blocks.iter().filter(result) {
                    let call = id(&block["tool_use_id"]);
                    // A result above its own call says the file is not what Claude Code wrote.
                    if last && made.contains(call) {
                        return unknown;
                    }
                    if last {
                        results.insert(call);
                    }
                    let tool = tools.get(call).copied();
                    // When the model waits for its scripts itself (`Poll`), the events come
                    // as that call's result, under a line that names their nonces.
                    let mut events = polled(&block["content"]).filter_map(settled);
                    match (receipt(&block["content"]), tool) {
                        (Some(n), Some("REPL")) if !ended.remove(&n) => _ = receipts.insert(call),
                        (None, Some("Poll")) => {
                            for event in events {
                                match event {
                                    Some(n) => _ = ended.insert(n),
                                    None => return unknown,
                                }
                            }
                        }
                        // The call is above the read's first line: whose result this is, and
                        // so what its text says, is not known.
                        (receipt, None) if receipt.is_some() || events.next().is_some() => {
                            return unknown;
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                end.get_or_insert(None);
                let message_id = &v["message"]["id"];
                if last && *message.get_or_insert_with(|| message_id.clone()) != *message_id {
                    (bounded, last) = (true, false);
                }
                for call in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let call_id = id(&call["id"]);
                    if last {
                        made.insert(call_id);
                    }
                    if last && !results.contains(call_id) {
                        calls.push(Call::of(call, v));
                    }
                    if receipts.remove(call_id) {
                        running.push(Call::of(call, v));
                    }
                }
            }
            _ => {}
        }
    }
    if !bounded || !receipts.is_empty() {
        return unknown;
    }
    let alive = |c: &Call| c.by == session && c.at.is_some_and(|at| at >= since);
    let stale = Waits::Unknown(Refused::Stale);
    let mut scripts: Vec<&Call> = Vec::new();
    for script in running.iter().filter(|c| c.asks().is_some()) {
        match script.at {
            Some(at) if at < since => {}
            _ => scripts.push(script),
        }
    }
    scripts.extend(calls.iter().filter(|c| c.asks().is_some()));
    let questions = || calls.iter().filter(|c| c.call["name"] == "AskUserQuestion");
    let call = match (&scripts[..], questions().count(), &calls[..]) {
        ([], 0, []) => {
            let quiet = running.iter().all(|c| c.at.is_some_and(|at| at < since));
            return match end.flatten().filter(|_| quiet) {
                Some(prompt) if !alive(&prompt) => stale,
                // A question in Code Mode is a script's, and a form still drawn when its
                // script has ended is asked no more: the screen stands for no call here.
                Some(_) if code_mode => Waits::Nothing,
                Some(_) if !moded => unknown,
                Some(prompt) => Waits::Open {
                    version: prompt.version,
                },
                None => Waits::Nothing,
            };
        }
        ([], 0, [one]) if alive(one) => return Waits::Other,
        ([script], 0, _) => *script,
        ([], 1, [one]) => one,
        ([], 0, [_]) => return stale,
        _ => return Waits::Unknown(Refused::Several),
    };
    if !alive(call) {
        return stale;
    }
    let Some(id) = call.call["id"].as_str() else {
        return unknown;
    };
    if let Some(code) = call.asks() {
        return Waits::Script {
            session: session.to_owned(),
            call: id.to_owned(),
            literals: literals(code),
        };
    }
    // When a dispatched script's question is answered, Claude Code writes the question as
    // an AskUserQuestion call of its own and then that call's result (2.1.329 ends that
    // message with `stop_sequence`). A read between the two lines sees a question with no
    // result that nobody is asked any more.
    if call.stop == "stop_sequence" {
        return unknown;
    }
    let questions = call.call["input"]["questions"]
        .as_array()
        .and_then(|questions| {
            let questions = questions.iter().map(|q| {
                let options = q["options"].as_array()?.iter().map(|o| {
                    let description = o["description"].as_str().unwrap_or_default();
                    Some((o["label"].as_str()?.to_owned(), description.to_owned()))
                });
                Some(Question {
                    text: q["question"].as_str()?.to_owned(),
                    options: options.collect::<Option<_>>()?,
                    multi: q["multiSelect"] == true,
                })
            });
            questions.collect::<Option<Vec<_>>>()
        });
    match questions {
        Some(questions) if !questions.is_empty() => Waits::Question {
            session: session.to_owned(),
            call: id.to_owned(),
            questions,
        },
        _ => unknown,
    }
}

/// The script number of a dispatch receipt: a tool result that reads `(dispatched #N)` and
/// nothing else, as a string or as its one text block.
fn receipt(content: &Value) -> Option<u64> {
    let text = match content {
        Value::String(text) => text,
        Value::Array(blocks) => match &blocks[..] {
            [block] if block["type"] == "text" => block["text"].as_str()?,
            _ => return None,
        },
        _ => return None,
    };
    let n = text.strip_prefix("(dispatched #")?.strip_suffix(')')?;
    n.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| n.parse().ok())?
}

/// The events of a `Poll` call's result: the lines that open one, in a result that starts
/// with the line Claude Code puts first (`<system>authentic event nonces ...`).
fn polled(content: &Value) -> impl Iterator<Item = &str> {
    let text = match content {
        Value::String(text) => text.as_str(),
        Value::Array(blocks) => match &blocks[..] {
            [block] => block["text"].as_str().unwrap_or_default(),
            _ => "",
        },
        _ => "",
    };
    let polled = text.starts_with("<system>authentic event nonces");
    text.lines()
        .filter(move |l| polled && l.starts_with("<event"))
}

/// The script a `poll_events` envelope says ended: an event whose opening tag reads
/// `kind="repl-eval"` and `eval="N"`. Only that tag is read: the event's body is the
/// script's output. `Some(None)` for such an event with no number.
fn settled(envelope: &str) -> Option<Option<u64>> {
    let tag = envelope.strip_prefix("<event")?;
    let tag = &tag[..tag.find(['>', '\n']).unwrap_or(tag.len())];
    let value = |name: &str| {
        let (_, rest) = tag.split_once(&format!(" {name}=\""))?;
        rest.split_once('"').map(|(value, _)| value)
    };
    (value("kind")? == "repl-eval").then(|| value("eval")?.parse().ok())
}

/// The string literals of a script, each the text between two matching quote characters
/// (`'`, `"` or a backtick) with `\n`, an escaped quote or backslash and `\u` escapes
/// undone; none when the script was not read whole. The script is not parsed, so a quote in
/// a comment or a regular expression opens a literal like any other and the read goes on out
/// of step, past literals it then never sees. A form is matched against every literal
/// ([`crate::prompt::Menu::scripted`] refuses one that a longer literal ends in), so a read
/// that may have lost one answers nothing: a literal a line break or the script's end leaves
/// open, which is where a stray quote shows, a template that holds `${`, whose text is not
/// what it gives and which can hold literals of its own, and an escape that is not undone.
fn literals(code: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = code.chars().peekable();
    while let Some(quote) = chars.next() {
        if !matches!(quote, '\'' | '"' | '`') {
            continue;
        }
        let mut text = String::new();
        loop {
            text.push(match chars.next()? {
                '\n' | '\r' if quote != '`' => return None,
                '$' if quote == '`' && chars.peek() == Some(&'{') => return None,
                c if c == quote => break,
                '\\' => match chars.next()? {
                    'n' => '\n',
                    c @ ('\'' | '"' | '`' | '\\') => c,
                    'u' => {
                        let digits: String = match chars.next_if_eq(&'{') {
                            Some(_) => chars.by_ref().take_while(|c| *c != '}').collect(),
                            None => chars.by_ref().take(4).collect(),
                        };
                        char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?
                    }
                    _ => return None,
                },
                c => c,
            });
        }
        out.push(text);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::prompt::{Menu, numbered};

    // Claude Code 2.1.329 in a terminal of 100 columns, captured 2026-11-20 (the plan file's
    // folder renamed). The screens are captures. Their transcripts are not what was on disk
    // while the form was up: each is the finished transcript cut at the call, reduced to the
    // fields read here. 2.1.329 writes a message that holds an AskUserQuestion call only
    // with the call's result (see the `live-` captures), so these stand for a Claude Code
    // that writes the call first, which the Bash capture shows it does for other tools. A
    // question; the first question of a call with two, its second, a multi-select one, the
    // same with a box checked, and the call's review page; a question of three lines, the
    // second a row of `─`, drawn behind the gutter; a question with four options in 12
    // rows, which scrolls, its cursor on option 1 and on option 4; a Bash prompt and the
    // plan prompt.
    macro_rules! cut {
        ($name:literal) => {
            (
                include_str!(concat!(
                    "../../tests/fixtures/claude-2.1.329/",
                    $name,
                    ".txt"
                )),
                include_str!(concat!(
                    "../../tests/fixtures/claude-2.1.329/",
                    $name,
                    ".transcript.cut.jsonl"
                )),
            )
        };
    }
    // A screen and the transcript read at that moment, reduced and its ids replaced.
    macro_rules! capture {
        ($name:literal) => {
            (
                include_str!(concat!(
                    "../../tests/fixtures/claude-2.1.329/",
                    $name,
                    ".txt"
                )),
                include_str!(concat!(
                    "../../tests/fixtures/claude-2.1.329/",
                    $name,
                    ".transcript.min.jsonl"
                )),
            )
        };
    }
    const ONE: (&str, &str) = cut!("question-one");
    const FIRST: (&str, &str) = cut!("two-questions-first");
    const MULTI: (&str, &str) = cut!("two-questions-second-multiselect");
    const CHECKED: (&str, &str) = cut!("two-questions-second-checked");
    const REVIEW: (&str, &str) = cut!("two-questions-after-tab");
    const MULTILINE: (&str, &str) = cut!("question-multiline");
    const TALL: (&str, &str) = cut!("question-tall-12-rows");
    const TALL_ON_4: (&str, &str) = cut!("question-tall-12-rows-cursor-4");
    const BASH: (&str, &str) = cut!("bash");
    const PLAN: (&str, &str) = cut!("plan");
    const FORMS: [(&str, &str); 6] = [ONE, FIRST, MULTI, CHECKED, REVIEW, MULTILINE];
    // The same Claude Code with Code Mode on (`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1`): the
    // model's one tool is `REPL`, and the question is a function its script calls. The form
    // is drawn as outside Code Mode. A question written in the call, and one whose text and
    // labels the script puts in variables first.
    const SCRIPTED: (&str, &str) = capture!("codemode-question-one");
    const SCRIPTED_BUILT: (&str, &str) = capture!("codemode-question-built");
    // Code Mode in auto mode (`--permission-mode auto`): Claude Code answers a `REPL` call
    // at once with `(dispatched #N)` and the script runs on. A question on screen, and one a
    // script asks after it computed and logged, each with its transcript at that moment;
    // then the transcript after the first was answered, and after the second was declined.
    // An attachment line keeps its type, and a `poll_events` one the first line of each
    // event, its nonce replaced.
    const DISPATCHED: (&str, &str) = capture!("dispatched-question-one");
    const DISPATCHED_LATER: (&str, &str) = capture!("dispatched-question-after-work");
    const ANSWERED: &str = include_str!(
        "../../tests/fixtures/claude-2.1.329/dispatched-question-answered.transcript.min.jsonl"
    );
    const DECLINED: &str = include_str!(
        "../../tests/fixtures/claude-2.1.329/dispatched-question-declined.transcript.min.jsonl"
    );
    const SESSION: &str = "00000000-0000-4000-8000-000000000000";

    /// A transcript read whole, each of `rows` a complete line.
    fn waits(rows: &str, whole: bool, since: u64) -> Waits {
        pending(&format!("{}\n", rows.trim_end()), whole, SESSION, since)
    }

    fn asked(transcript: &str) -> Option<Vec<Question>> {
        match waits(transcript, true, 0) {
            Waits::Question { questions, .. } => Some(questions),
            _ => None,
        }
    }

    fn says(waits: Waits) -> String {
        match waits {
            Waits::Question { questions, .. } => format!("{} asked", questions.len()),
            Waits::Script { .. } => "a script".to_owned(),
            Waits::Other => "another call".to_owned(),
            Waits::Open { version } => format!("open {version:?}"),
            Waits::Nothing => "nothing".to_owned(),
            Waits::Unknown(why) => why.as_str().to_owned(),
        }
    }

    /// The options of a screen that `Approvals::screen` answers beside a pending call.
    fn offered(screen: &str, asked: &[Question]) -> Option<Vec<String>> {
        Menu::parse(screen)
            .filter(|m| m.on_screen() && m.asks(asked).is_ok())
            .map(|m| m.heads)
    }

    #[test]
    fn a_captured_form_offers_the_question_its_transcript_asks() {
        let heads = [
            vec![
                "SQLite",
                "Redis",
                "Memory",
                "Type something.",
                "Chat about this",
            ],
            vec!["SQLite", "Redis", "Type something.", "Chat about this"],
            vec![
                "[ ] Lint",
                "[ ] Tests",
                "[ ] Schema",
                "[ ] Type something",
                "Chat about this",
            ],
            vec![
                "[✔] Lint",
                "[ ] Tests",
                "[ ] Schema",
                "[ ] Type something",
                "Chat about this",
            ],
            vec!["Submit answers", "Cancel"],
            vec!["File", "SQLite", "Type something.", "Chat about this"],
        ];
        for ((screen, transcript), heads) in FORMS.iter().zip(heads) {
            let call = asked(transcript).unwrap();
            assert_eq!(offered(screen, &call).unwrap(), heads, "{screen}");
            for (_, other) in FORMS {
                let other = asked(other).unwrap();
                assert!(
                    other == call || offered(screen, &other).is_none(),
                    "{screen}"
                );
            }
        }
        let bound = |(screen, transcript): (&str, &str)| {
            Menu::parse(screen)
                .unwrap()
                .asks(&asked(transcript).unwrap())
        };
        assert_ne!(bound(FIRST), bound(MULTI), "each question of one call");
        assert_ne!(bound(MULTI), bound(REVIEW));
        assert_eq!(bound(MULTI), bound(CHECKED));
        for (screen, transcript) in [BASH, PLAN] {
            assert_eq!(waits(transcript, true, 0), Waits::Other, "{screen}");
            let menu = Menu::parse(screen).unwrap();
            assert!(!menu.is_question(), "{screen}");
        }
    }

    #[test]
    fn a_form_cut_at_any_height_offers_nothing() {
        for (screen, transcript) in FORMS {
            let call = asked(transcript).unwrap();
            let rows: Vec<&str> = screen.lines().collect();
            let header = rows.iter().position(|r| r.contains(['☐', '☒'])).unwrap();
            assert!(crate::prompt::is_rule(rows[header - 1]));
            for from in 0..rows.len() {
                assert_eq!(
                    offered(&rows[from..].join("\n"), &call).is_some(),
                    from < header,
                    "from row {from} of {screen}"
                );
            }
        }
    }

    #[test]
    fn a_form_that_scrolls_offers_nothing() {
        for (screen, transcript) in [TALL, TALL_ON_4] {
            assert!(asked(transcript).is_some());
            assert_eq!(Menu::parse(screen), None, "{screen}");
            assert!(numbered(screen), "{screen}");
        }
        // Hand-built from the capture, not captured: every option of a question of two on
        // screen under a header and a rule, and under the last one the row that says rows are
        // hidden.
        let scrolled = "\
────────────────────────────────────────────────────────────
 ☐ Channel

Which release channel should this build go to?

❯ 1. Stable
  2. Beta
  3. Type something.
 ↑ 1 more line
Enter to select · ↑/↓ to navigate · Esc to cancel
";
        let mut call = asked(TALL.1).unwrap();
        call[0].options.truncate(2);
        for (_, description) in &mut call[0].options {
            description.clear();
        }
        assert_eq!(offered(scrolled, &call), None);
        assert!(offered(&scrolled.replace(" ↑ 1 more line\n", "\n"), &call).is_some());
    }

    #[test]
    fn a_rule_the_question_draws_never_stands_for_the_top() {
        // Hand-built from the capture, not captured: the question over three rows at column 0,
        // as Claude Code drew one before the gutter, the second a row of `─`. Behind the gutter
        // the same row is no rule: MULTILINE holds one and is offered whole.
        let (screen, transcript) = ONE;
        let shown = "Which cache backend should we use?";
        let text = format!(
            "Drop the production database first.\n{}\n ☐ Cache\n{shown}",
            "─".repeat(30)
        );
        let drawn = screen.replacen(&format!("\n{shown}\n"), &format!("\n{text}\n"), 1);
        assert_ne!(drawn, screen);
        let mut call = asked(transcript).unwrap();
        assert!(
            offered(&drawn, &call).is_some(),
            "the rows under that row read as the whole form of a shorter question"
        );
        call[0].text = text;
        let rows: Vec<&str> = drawn.lines().collect();
        for from in 0..rows.len() {
            assert_eq!(
                offered(&rows[from..].join("\n"), &call),
                None,
                "from row {from}"
            );
        }

        // Behind the gutter, a first row that reads `│` and is not on screen: the row left
        // is not read without its gutter to make up for it.
        let (screen, transcript) = MULTILINE;
        let mut call = asked(transcript).unwrap();
        let (from, last) = ("│ We need", "│ Which store should it use?");
        let (head, rest) = screen.split_once(from).unwrap();
        let (_, tail) = rest.split_once(last).unwrap();
        call[0].text = "│\nWhich store should it use?".into();
        assert!(offered(&format!("{head}│ │\n{last}{tail}"), &call).is_some());
        assert_eq!(offered(&format!("{head}{last}{tail}"), &call), None);
    }

    #[test]
    fn two_questions_with_the_same_options_are_refused() {
        // Hand-built from the capture, not captured: the first question's text ends in a row
        // of `─`, a copy of the tabs and the second question's text, so the rows under that
        // row read as the whole second question.
        let (screen, transcript) = FIRST;
        let shown = "Which cache backend should we use?";
        let text = format!(
            "Wipe the disk first?\n{}\n←  ☒ Wipe  ☐ Cache  ✔ Submit  →\n{shown}",
            "─".repeat(30)
        );
        let drawn = screen.replacen(&format!("\n{shown}\n"), &format!("\n{text}\n"), 1);
        assert_ne!(drawn, screen);
        let mut call = asked(transcript).unwrap();
        call[1] = call[0].clone();
        call[0].text = text.clone();
        assert!(offered(&drawn, &call[1..]).is_some(), "reads as the second");
        assert_eq!(offered(&drawn, &call), None);
        call[0].text = "Which cache backend comes first?".into();
        assert_eq!(offered(screen, &call), None, "drawn whole too");

        // The same spoof with the first question's options worded as the second's, the words
        // split otherwise between label and description: the screen draws both alike.
        let (label, description) = call[1].options[0].clone();
        let (moved, rest) = description.split_once(' ').unwrap();
        call[0].text = text;
        call[0].options[0] = (format!("{label} {moved}"), rest.to_owned());
        assert_ne!(call[0].options, call[1].options);
        assert!(offered(&drawn, &call[1..]).is_some());
        let menu = Menu::parse(&drawn).unwrap();
        assert_eq!(menu.asks(&call), Err(Refused::Ambiguous));
    }

    #[test]
    fn only_the_last_turns_one_call_without_a_result_is_pending() {
        let at = "2026-11-20T11:35:11.762Z";
        let then = timestamp_ms(at).unwrap();
        assert_eq!(says(waits(ONE.1, true, then)), "1 asked");
        assert_eq!(
            says(waits(ONE.1, true, then + 1)),
            "stale call",
            "older than the process"
        );
        let other = "11111111-0000-4000-8000-000000000000";
        assert_eq!(
            says(pending(ONE.1, true, other, 0)),
            "stale call",
            "another session's"
        );
        let open = "open Some([2, 1, 329])";
        // The first call was declined, a prompt followed, and the second call is pending.
        let rows: Vec<&str> = FIRST.1.lines().collect();
        let upto = |n: usize| says(waits(&rows[..n].join("\n"), true, 0));
        assert_eq!(
            [upto(2), upto(3), upto(4), upto(5), upto(6)],
            ["1 asked", open, "nothing", open, "2 asked"],
            "the result of a declined call, the line that says it was interrupted, a prompt"
        );

        let call = |id: &str, message: &str, name: &str| {
            let question = json!({"question": "Q?", "options": [{"label": "A"}, {"label": "B"}]});
            json!({"type": "assistant", "timestamp": at, "sessionId": SESSION, "message": {
                "id": message,
                "content": [{"type": "tool_use", "id": id, "name": name,
                    "input": {"questions": [question]}}],
            }})
            .to_string()
        };
        let ask = |id: &str, message: &str| call(id, message, "AskUserQuestion");
        let user = |content: Value| {
            json!({"type": "user", "timestamp": at, "sessionId": SESSION,
                "version": "2.1.329", "message": {"content": content}})
            .to_string()
        };
        let result = |id: &str| user(json!([{"type": "tool_result", "tool_use_id": id}]));
        let prompt = || user(json!("go"));
        let side = json!({"type": "user", "isSidechain": true, "message": {"content": "go"}});
        let reads = |rows: &[String], whole| says(waits(&rows.join("\n"), whole, 0));
        assert_eq!(
            reads(&[prompt(), ask("a", "m1"), side.to_string()], true),
            "1 asked"
        );
        assert_eq!(
            reads(&[prompt(), call("a", "m1", "ExitPlanMode")], true),
            "another call"
        );
        assert_eq!(
            reads(&[ask("a", "m1"), prompt()], true),
            open,
            "a prompt since"
        );
        let several = "several pending calls";
        assert_eq!(
            reads(&[prompt(), ask("a", "m1"), prompt(), ask("c", "m1")], true),
            several,
            "two questions, a line that is no result between them"
        );
        assert_eq!(
            reads(&[prompt(), call("b", "m1", "Bash"), ask("a", "m1")], true),
            several,
            "a Bash call beside it"
        );
        let read = [call("b", "m1", "Read"), result("b")];
        assert_eq!(
            reads(
                &[&[prompt(), ask("a", "m1")], &read[..], &[ask("c", "m1")]].concat(),
                true
            ),
            several,
            "two questions, a result between them"
        );
        assert_eq!(
            reads(&[&[prompt()], &read[..], &[ask("a", "m1")]].concat(), true),
            "1 asked"
        );
        // A read that starts inside the turn: bounded by another message's line, or not.
        let unknown = "transcript unreadable or incomplete";
        let earlier = [call("b", "m0", "Read"), result("b")];
        assert_eq!(
            reads(&[&earlier[..], &[ask("a", "m1")]].concat(), false),
            "1 asked"
        );
        assert_eq!(
            reads(&[&read[..], &[ask("a", "m1")]].concat(), false),
            unknown
        );
        assert_eq!(
            reads(&[&read[..], &[ask("a", "m1")]].concat(), true),
            "1 asked"
        );

        // A result above the call it answers: not a pending call.
        assert_eq!(
            reads(&[prompt(), result("a"), ask("a", "m1")], true),
            unknown
        );
        assert_eq!(
            reads(&[prompt(), result("b"), ask("a", "m1")], true),
            "1 asked"
        );

        // A last line that Claude Code is still writing, the call's result or a second call:
        // nothing is known, whether the cut line has its newline yet or not.
        let asked = [prompt(), ask("a", "m1")].join("\n");
        for (next, whole) in [(result("a"), open), (ask("c", "m1"), several)] {
            let cut = &next[..next.len() / 2];
            let read = |text: String| says(pending(&text, true, SESSION, 0));
            assert_eq!(read(format!("{asked}\n{next}\n")), whole);
            assert_eq!(read(format!("{asked}\n{cut}")), unknown);
            assert_eq!(read(format!("{asked}\n{cut}\n")), unknown);
            assert_eq!(read(format!("{asked}\n{next}")), unknown);
        }
    }
    fn code(transcript: &str) -> String {
        let last: Value = serde_json::from_str(transcript.lines().last().unwrap()).unwrap();
        let call = &last["message"]["content"][0];
        assert_eq!(call["name"], "REPL");
        call["input"]["code"].as_str().unwrap().to_owned()
    }

    /// The options of a screen that `Approvals::screen` answers beside a pending script.
    fn scripts(screen: &str, code: &str) -> Result<Vec<String>, Refused> {
        let menu = Menu::parse(screen).filter(Menu::on_screen);
        let menu = menu.ok_or(Refused::NotWhole)?;
        let literals = literals(code).ok_or(Refused::ScriptNotRead)?;
        menu.scripted(&literals).map(|_| menu.heads)
    }

    #[test]
    fn a_scripts_literals_are_read_whole_or_not_at_all() {
        let code = r#"f('x', "y\n\"z", `t`, 'it\'s', '\u00e9\u{1F600}')"#;
        let read = ["x", "y\n\"z", "t", "it's", "é😀"].map(str::to_owned);
        assert_eq!(literals(code), Some(read.to_vec()));
        // What may have lost a literal: a substitution, an escape that is not undone (a tab, a
        // surrogate), a quote a line break, a carriage return or the script's end leaves open.
        for lost in [
            "`u${v}`",
            "'w\\tq'",
            "'\\uD83D\\uDE00'",
            "// don't\n'k'",
            "'a\rb'",
            "'un",
            "`un",
        ] {
            assert_eq!(literals(&format!("{code}; {lost}")), None, "{lost}");
        }
    }

    #[test]
    fn a_code_mode_question_is_matched_to_the_pending_scripts_literals() {
        let asked = ["Type something.", "Chat about this"];
        for ((screen, transcript), labels) in [
            (SCRIPTED, ["Option A", "Option B"]),
            (SCRIPTED_BUILT, ["Red", "Blue"]),
        ] {
            assert!(says(waits(transcript, true, 0)).starts_with("a script"));
            let heads = scripts(screen, &code(transcript)).unwrap();
            assert_eq!(heads, [&labels[..], &asked[..]].concat(), "{screen}");
            // From any row down, as a terminal too short would show it.
            let rows: Vec<&str> = screen.lines().collect();
            let header = rows.iter().position(|r| r.contains('☐')).unwrap();
            for from in 0..rows.len() {
                let cut = rows[from..].join("\n");
                assert_eq!(
                    scripts(&cut, &code(transcript)).is_ok(),
                    from < header,
                    "from row {from} of {screen}"
                );
            }
        }
        let (screen, transcript) = SCRIPTED;
        let code = code(transcript);
        let not_found = Err(Refused::NotInScript);
        assert_eq!(scripts(screen, &self::code(SCRIPTED_BUILT.1)), not_found);
        // A pending script that names no question is another call.
        let silent = transcript.replace("AskUserQuestion", "Bash");
        assert_eq!(says(waits(&silent, true, 0)), "another call");

        // What the form draws must each be a whole literal: not the question split in two, in
        // a template that holds a substitution, or behind an escape that is not undone, and
        // not a label built from pieces.
        let asks = "'Which option do you want to pick for this test?'";
        for written in [
            "'Which option do you want ' + 'to pick for this test?'",
            "'Which option do you want to pick for this test? Think it over.'",
            "'Say: Which option do you want to pick for this test? Now.'",
        ] {
            assert_ne!(code.replace(asks, written), code);
            assert_eq!(scripts(screen, &code.replace(asks, written)), not_found);
        }
        // A literal that cannot be compared as written is none: a template's text is not what
        // its substitution gives, nor an escape's letter what the escape gives. Each of these
        // screens shows the literal's own characters.
        let shown = "Which option do you want to pick for this test?";
        for (drawn, written, quoted) in [
            (
                "Pay ${amount} now?",
                "`Pay ${amount} now?`",
                "'Pay ${amount} now?'",
            ),
            ("Use atb mode?", "'Use a\\tb mode?'", "'Use atb mode?'"),
        ] {
            let screen = screen.replacen(&format!("\n{shown}\n"), &format!("\n{drawn}\n"), 1);
            let unread = Err(Refused::ScriptNotRead);
            assert_eq!(scripts(&screen, &code.replace(asks, written)), unread);
            assert!(scripts(&screen, &code.replace(asks, quoted)).is_ok());
        }
        let built = code.replace("'Option A'", "'Option ' + 'A'");
        assert_ne!(built, code);
        assert_eq!(scripts(screen, &built), not_found);
        // An option is a label and a description, each a whole literal and each on rows of
        // its own: not a label alone, nor one under a description the script does not hold,
        // and not the same characters split elsewhere.
        let bare = screen.replacen("     First test choice.\n", "", 1);
        assert_ne!(bare, screen);
        assert_eq!(scripts(&bare, &code), not_found);
        let other = code.replace("'First test choice.'", "'Another choice.'");
        assert_ne!(other, code);
        assert_eq!(scripts(screen, &other), not_found);
        let split = code
            .replace("'Option A'", "'Option'")
            .replace("'First test choice.'", "'A First test choice.'");
        assert!(
            literals(&split)
                .unwrap()
                .contains(&"A First test choice.".to_owned())
        );
        assert_eq!(scripts(screen, &split), not_found);
    }

    #[test]
    fn a_cut_form_in_code_mode_is_no_whole_literal() {
        // Hand-built from the capture, not captured: the question over rows at column 0, as
        // Claude Code drew one before the gutter, one of them a row of `─` and the next a
        // copy of the header, so the rows under that row read as a whole, shorter form.
        let (screen, transcript) = SCRIPTED;
        let shown = "Which option do you want to pick for this test?";
        let rule = "─".repeat(30);
        let text = format!("Drop the production database first.\n{rule}\n ☐ Test\n{shown}");
        let drawn = screen.replacen(&format!("\n{shown}\n"), &format!("\n{text}\n"), 1);
        assert_ne!(drawn, screen);
        let short = code(transcript);
        let long = short.replace(shown, &text.replace('\n', "\\n"));
        assert_ne!(long, short);
        assert!(literals(&long).unwrap().contains(&text));
        // A script that asks only the short question: the screen is its whole form.
        assert!(scripts(&drawn, &short).is_ok());
        // The script asks the long one: what shows is the end of a literal and none itself.
        let rows: Vec<&str> = drawn.lines().collect();
        for from in 0..rows.len() {
            let cut = rows[from..].join("\n");
            assert!(scripts(&cut, &long).is_err(), "from row {from}");
        }
        assert_eq!(scripts(&drawn, &long), Err(Refused::NotWhole));
        // It holds that end as a literal of its own too, as a second question would: refused
        // all the same, since a longer literal ends in it.
        let both = format!("{long}\n{short}");
        assert_eq!(scripts(&drawn, &both), Err(Refused::NotWhole));
        assert!(
            scripts(screen, &both).is_err(),
            "the short one drawn whole too"
        );

        // The same script with the long literal where a read that does not parse loses it:
        // after an apostrophe or an open quote in a comment on its line, behind an escape that
        // is not undone, and inside a template's substitution. The short literal is still
        // read, and without the long one nothing says the form is cut: a script not read
        // whole answers nothing.
        let quoted = text.replace('\n', "\\n");
        let tabbed = long.replace("Drop the", "Drop\\tthe");
        assert_ne!(tabbed, long);
        for unread in [
            format!("/* don't translate */ {long}\n{short}"),
            format!("/* say \"hi */ {long}\n{short}"),
            format!("{tabbed}\n{short}"),
            format!("const asked = `${{'{quoted}'}}`;\n{short}"),
        ] {
            assert_eq!(
                scripts(&drawn, &unread),
                Err(Refused::ScriptNotRead),
                "{unread}"
            );
        }
    }

    #[test]
    fn a_question_made_of_pieces_is_refused_by_the_screen_when_cut() {
        // Hand-built from the capture, not captured: a script that makes its question of a
        // warning, a rule and `Continue?`, the last a whole literal. A terminal that showed
        // only that last row would match it. What refuses it is the screen: cut under the
        // rule the dialog has no top, and whole, the rows under the rule are less than a
        // header and a text; behind the gutter, as 2.1.329 draws a question of several rows,
        // that rule is no top at all.
        let (screen, transcript) = SCRIPTED;
        let asks = "'Which option do you want to pick for this test?'";
        let pieces = "'Warning: this deletes the cache.\\n' + '─'.repeat(30) + '\\nContinue?'";
        let code = code(transcript).replace(asks, pieces);
        assert!(literals(&code).unwrap().contains(&"\nContinue?".to_owned()));
        let shown = "\nWhich option do you want to pick for this test?\n";
        let rule = "─".repeat(30);
        for gutter in ["", "│ "] {
            let rows = ["Warning: this deletes the cache.", &rule, "Continue?"];
            let rows: Vec<String> = rows.iter().map(|r| format!("{gutter}{r}")).collect();
            let drawn = screen.replacen(shown, &format!("\n{}\n", rows.join("\n")), 1);
            assert_ne!(drawn, screen);
            let all: Vec<&str> = drawn.lines().collect();
            for from in 0..all.len() {
                let cut = all[from..].join("\n");
                assert!(scripts(&cut, &code).is_err(), "{gutter}from row {from}");
            }
        }
    }

    #[test]
    fn a_script_of_one_literal_many_times_is_matched_as_any_other() {
        // A megabyte of `'a',` and an option that starts with it: the literals are a set,
        // looked up once for each way an option's rows split into label and description.
        let (screen, transcript) = SCRIPTED;
        let code = format!("[{}];\n{}", "'a',".repeat(200_000), code(transcript));
        let drawn = screen.replacen("1. Option A", "1. a Option A", 1);
        assert_ne!(drawn, screen);
        assert_eq!(scripts(&drawn, &code), Err(Refused::NotInScript));
        assert!(scripts(screen, &code).is_ok());
    }

    fn repl(id: &str, code: &str) -> String {
        json!({"type": "assistant", "timestamp": "2026-11-20T17:25:01.028Z", "sessionId": SESSION,
        "message": {"content": [
            {"type": "tool_use", "id": id, "name": "REPL", "input": {"code": code}},
        ]}})
        .to_string()
    }

    fn answer(id: &str, content: Value) -> String {
        let block = json!({"type": "tool_result", "tool_use_id": id, "content": content});
        json!({"type": "user", "timestamp": "2026-11-20T17:25:01.500Z", "sessionId": SESSION,
            "version": "2.1.329", "message": {"content": [block]}})
        .to_string()
    }

    fn events(envelopes: &[&str]) -> String {
        let attachment = json!({"type": "poll_events", "envelopes": envelopes});
        json!({"type": "attachment", "attachment": attachment}).to_string()
    }

    fn ended(eval: u32) -> String {
        events(&[&format!(
            r#"<event nonce="n" kind="repl-eval" at="t" eval="{eval}" ms="1" status="settled">"#
        )])
    }

    /// The options a screen is answered with beside a transcript, and the call it is bound to.
    fn runs(screen: &str, transcript: &str) -> Option<(Vec<String>, String)> {
        let Waits::Script { call, literals, .. } = waits(transcript, true, 0) else {
            return None;
        };
        let menu = Menu::parse(screen).filter(Menu::on_screen)?;
        menu.scripted(&literals?).ok().map(|_| (menu.heads, call))
    }

    // Claude Code 2.1.329 on 2026-11-20, each screen with the transcript read while the form
    // was up (100 columns; reduced, ids and nonces replaced). Outside Code Mode: a question
    // on a session's first prompt in auto mode and in manual mode; one on a third prompt,
    // after a declined question and a text turn; one asked after a Bash call in the same
    // turn, and one after a sentence in the same message; the two questions of one call, a
    // box checked, and the review page; a question of one line of 96 columns, and one of
    // three lines, each behind the gutter; a question in 12 rows. In none is the call on
    // disk. A Bash permission prompt in manual mode, whose call is. In Code Mode: a
    // question in manual mode, its `REPL` call with no result; one in auto mode beside the
    // model's own `Poll`, and the transcript after its answer, the event in `Poll`'s result.
    const LIVE: [(&str, &str); 9] = [
        capture!("live-question"),
        capture!("live-question-manual"),
        capture!("live-question-later"),
        capture!("live-question-after-tool"),
        capture!("live-question-after-text"),
        capture!("live-two-questions-first"),
        capture!("live-two-questions-second"),
        capture!("live-two-questions-second-checked"),
        capture!("live-question-wide"),
    ];
    const LIVE_MULTILINE: (&str, &str) = capture!("live-question-multiline");
    const LIVE_REVIEW: (&str, &str) = capture!("live-two-questions-review");
    const LIVE_TALL: [(&str, &str); 2] = [
        capture!("live-question-tall"),
        capture!("live-question-tall-cursor-4"),
    ];
    const LIVE_BASH: (&str, &str) = capture!("live-bash-manual");
    const LIVE_SCRIPTED: (&str, &str) = capture!("live-scripted-manual");
    const LIVE_POLL: (&str, &str) = capture!("live-scripted-poll");
    const LIVE_POLL_ANSWERED: &str = include_str!(
        "../../tests/fixtures/claude-2.1.329/live-scripted-poll-answered.transcript.min.jsonl"
    );

    #[test]
    fn a_question_outside_code_mode_leaves_no_call_and_an_open_turn() {
        let open = "open Some([2, 1, 329])";
        let whole =
            |screen: &str| Menu::parse(screen).is_some_and(|m| m.on_screen() && m.stands_whole());
        for (screen, transcript) in LIVE.into_iter().chain([LIVE_MULTILINE]) {
            assert_eq!(says(waits(transcript, true, 0)), open, "{screen}");
            assert!(whole(screen), "{screen}");
        }
        // The review page and a form that scrolls are not shown whole by the screen.
        for (screen, transcript) in LIVE_TALL.into_iter().chain([LIVE_REVIEW]) {
            assert_eq!(says(waits(transcript, true, 0)), open, "{screen}");
            assert!(!whole(screen), "{screen}");
        }
        assert_eq!(says(waits(LIVE_BASH.1, true, 0)), "another call");
        assert_eq!(says(waits(LIVE_SCRIPTED.1, true, 0)), "a script");
        assert!(runs(LIVE_SCRIPTED.0, LIVE_SCRIPTED.1).is_some());
        // Beside the model's own Poll the script is the state; its event, in Poll's result,
        // ends it.
        let (heads, _) = runs(LIVE_POLL.0, LIVE_POLL.1).unwrap();
        assert_eq!(heads[..2], ["Carry on", "Stop here"]);
        assert_eq!(says(waits(LIVE_POLL_ANSWERED, true, 0)), "nothing");
        let rows: Vec<&str> = LIVE_POLL_ANSWERED.lines().collect();
        let poll = rows
            .iter()
            .position(|r| r.contains("authentic event nonces"))
            .unwrap();
        let upto = |n: usize| says(waits(&rows[..n].join("\n"), true, 0));
        assert_eq!([upto(poll), upto(poll + 1)], ["a script", "nothing"]);

        // The third prompt's transcript, line by line: the turn is open at a prompt, and
        // over once Claude Code wrote that a call was interrupted or a reply ended.
        let rows: Vec<&str> = LIVE[2].1.lines().collect();
        let at = |n: usize| says(waits(&rows[..n].join("\n"), true, 0));
        let line = |part: &str| rows.iter().rposition(|r| r.contains(part)).unwrap() + 1;
        let (declined, replied) = (
            line("[Request interrupted"),
            line(r#""stop_reason":"end_turn""#),
        );
        assert!(declined < replied && replied < rows.len());
        assert_eq!(
            [at(declined), at(replied), at(rows.len())],
            ["nothing", "nothing", open]
        );
        // A prompt older than the process, or another session's, is not this turn.
        let stamp = timestamp_ms("2026-11-20T23:59:59.000Z").unwrap();
        assert_eq!(says(pending(LIVE[0].1, true, SESSION, stamp)), "stale call");
        let other = "11111111-0000-4000-8000-000000000000";
        assert_eq!(says(pending(LIVE[0].1, true, other, 0)), "stale call");
        // A script still running, whatever it does, is not a quiet transcript.
        let quiet = [
            repl("b", "await build()"),
            answer("b", json!("(dispatched #1)")),
        ];
        let prompt = json!({"type": "user", "timestamp": "2026-11-20T17:25:02.000Z",
            "sessionId": SESSION, "version": "2.1.329", "message": {"content": "go"}});
        let reads = |rows: &[String]| says(waits(&rows.join("\n"), true, 0));
        assert_eq!(reads(&[prompt.to_string()]), open);
        assert_eq!(
            reads(&[&quiet[..], &[prompt.to_string()]].concat()),
            "nothing"
        );
    }

    #[test]
    fn a_read_that_starts_inside_a_line_drops_that_line() {
        // A transcript longer than the read: the read's first line is cut and is no line.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let line = |content: String| json!({"type": "user", "message": {"content": content}});
        let long = line("x".repeat(TAIL_BYTES as usize));
        let prompt = line("go".to_owned());
        let asked = DISPATCHED.1.trim_end();
        std::fs::write(&path, format!("{long}\n{prompt}\n{asked}\n")).unwrap();
        let asking = Asking {
            path: path.clone(),
            session: SESSION.to_owned(),
            started_at: 0,
        };
        assert_eq!(says(asking.waits()), "a script");
        // With no line above the script's message that says where it starts, the read does
        // not reach far enough to know.
        std::fs::write(&path, format!("{long}\n{asked}\n")).unwrap();
        assert_eq!(says(asking.waits()), "transcript unreadable or incomplete");
    }

    #[test]
    fn a_dispatched_script_runs_until_an_event_names_it() {
        let own = ["Type something.", "Chat about this"].map(str::to_owned);
        let (heads, call) = runs(DISPATCHED.0, DISPATCHED.1).unwrap();
        assert_eq!(heads[..4], ["Jupiter", "Saturn", "Neptune", "Earth"]);
        assert_eq!((&heads[4..], &*call), (&own[..], "toolu_scrubbed_1"));
        let (heads, call) = runs(DISPATCHED_LATER.0, DISPATCHED_LATER.1).unwrap();
        assert_eq!(heads[..2], ["Yes please", "Stop here"]);
        assert_eq!((&heads[2..], &*call), (&own[..], "toolu_scrubbed_3"));
        assert_eq!(
            runs(DISPATCHED.0, DISPATCHED_LATER.1),
            None,
            "the script before it ended"
        );
        for ended in [ANSWERED, DECLINED] {
            assert_eq!(says(waits(ended, true, 0)), "nothing");
        }

        // The answered transcript, line by line: the call, its receipt, the event that ends
        // the script, the question written as a call of its own, that call's result.
        let unknown = "transcript unreadable or incomplete";
        // Once its script has ended a session in Code Mode waits on nothing: the screen
        // stands for no call there.
        let over = "nothing";
        let rows: Vec<&str> = ANSWERED.lines().collect();
        let upto = |n: usize| says(waits(&rows[..n].join("\n"), true, 0));
        assert!(rows[5].contains(r#""name":"REPL""#) && rows[8].contains("poll_events"));
        assert_eq!(
            [
                upto(6),
                upto(7),
                upto(8),
                upto(9),
                upto(10),
                upto(11),
                upto(12)
            ],
            [
                "a script", "a script", "a script", over, unknown, over, over
            ],
            "between the question's own call and its result nothing is asked any more"
        );

        let asks = "await tools.AskUserQuestion({questions: []})";
        let receipt = |n: u32| json!(format!("(dispatched #{n})"));
        let reads = |rows: &[String]| says(waits(&rows.join("\n"), true, 0));
        let one = [repl("a", asks), answer("a", receipt(1))];
        assert_eq!(reads(&one), "a script");
        // Only an event for that script, under its receipt, ends it. The numbers start again
        // in a new context: an event counts for the nearest receipt above it.
        assert_eq!(reads(&[&one[..], &[ended(2)]].concat()), "a script");
        assert_eq!(reads(&[&one[..], &[ended(1)]].concat()), over);
        assert_eq!(reads(&[&[ended(1)], &one[..]].concat()), "a script");
        let again = [repl("b", "1 + 1"), answer("b", receipt(1)), ended(1)];
        assert_eq!(reads(&[&one[..], &again[..]].concat()), "a script");
        // An event is read from its opening tag: its body is the script's output.
        let quoted = r#"<event nonce="n" kind="repl-eval" eval="7">\n kind="repl-eval" eval="1""#;
        let status = r#"<event nonce="n">\n kind="repl-eval" eval="1""#;
        assert_eq!(
            reads(&[&one[..], &[events(&[quoted, status])]].concat()),
            "a script"
        );
        let unnamed = r#"<event nonce="n" kind="repl-eval" status="settled">"#;
        assert_eq!(reads(&[&one[..], &[events(&[unnamed])]].concat()), unknown);

        // A receipt is that text and nothing else, as a string or as its one text block.
        let block = json!([{"type": "text", "text": "(dispatched #1)"}]);
        assert_eq!(reads(&[repl("a", asks), answer("a", block)]), "a script");
        for other in [
            json!("(dispatched #1) "),
            json!("(dispatched #1)\n4"),
            json!("dispatched #1"),
            json!("(dispatched #one)"),
            json!("(dispatched #)"),
            json!([{"type": "text", "text": "(dispatched #1)"}, {"type": "text", "text": "4"}]),
        ] {
            assert_eq!(
                reads(&[repl("a", asks), answer("a", other.clone())]),
                over,
                "{other}"
            );
        }
        // A receipt whose call the read does not hold.
        assert_eq!(reads(&one[1..]), unknown);

        // One asking script, whatever else runs or waits beside it; two are one too many.
        let quiet = [repl("b", "await build()"), answer("b", receipt(2))];
        let second = [repl("b", asks), answer("b", receipt(2))];
        let waiting = |name: &str| {
            let call = json!({"type": "tool_use", "id": "c", "name": name, "input": {}});
            json!({"type": "assistant", "timestamp": "2026-11-20T17:25:02.000Z",
                "sessionId": SESSION, "message": {"content": [call]}})
            .to_string()
        };
        let several = "several pending calls";
        assert_eq!(reads(&[&one[..], &quiet[..]].concat()), "a script");
        assert_eq!(reads(&[&one[..], &second[..]].concat()), several);
        assert_eq!(reads(&[&one[..], &[repl("b", asks)]].concat()), several);
        for name in ["Poll", "Bash"] {
            assert_eq!(reads(&[&one[..], &[waiting(name)]].concat()), "a script");
            assert_eq!(
                reads(&[&quiet[..], &[waiting(name)]].concat()),
                "another call"
            );
        }
        assert_eq!(
            reads(&[&one[..], &[waiting("AskUserQuestion")]].concat()),
            several
        );

        // A script older than the process died with the one that ran it; one of another
        // session is not known to have.
        let at = timestamp_ms("2026-11-20T17:25:01.028Z").unwrap();
        let read = |since: u64| {
            says(pending(
                &format!("{}\n", one.join("\n")),
                true,
                SESSION,
                since,
            ))
        };
        assert_eq!([read(at), read(at + 1)], ["a script", over]);
        let other = one
            .join("\n")
            .replace(SESSION, "11111111-0000-4000-8000-000000000000");
        assert_eq!(says(waits(&other, true, 0)), "stale call");
    }
    #[test]
    fn the_screen_stands_for_a_call_only_where_the_read_shows_no_code_mode() {
        let open = "open Some([2, 1, 329])";
        let unknown = "transcript unreadable or incomplete";
        let prompt = json!({"type": "user", "timestamp": "2026-11-20T17:25:02.000Z",
            "sessionId": SESSION, "version": "2.1.329", "message": {"content": "go"}})
        .to_string();
        let mode = json!({"type": "permission-mode", "permissionMode": "auto"}).to_string();
        let scripts = json!({"type": "repl-mode", "replMode": true}).to_string();
        let read = |whole: bool, rows: &[&str]| says(waits(&rows.join("\n"), whole, 0));
        // A read from the file's start holds every call the session made. One that starts
        // inside the file shows the mode by the records Claude Code writes through a
        // session, or shows nothing of it.
        assert_eq!(read(true, &[&prompt]), open);
        assert_eq!(read(false, &[&prompt]), unknown);
        assert_eq!(read(false, &[&mode, &prompt]), open);
        for whole in [true, false] {
            assert_eq!(read(whole, &[&mode, &scripts, &prompt]), "nothing");
        }
        // A script that ran says as much: a form still drawn when its script has ended, or
        // drawn by one above the read, is not the screen's to answer.
        let (call, result) = (repl("a", "1 + 1"), answer("a", json!("2")));
        assert_eq!(read(true, &[&call, &result, &prompt]), "nothing");
    }

    #[test]
    fn a_record_that_is_no_prompt_opens_no_turn() {
        let line = |marks: Value, content: Value| {
            let mut line = json!({"type": "user", "timestamp": "2026-11-20T17:25:03.000Z",
                "sessionId": SESSION, "version": "2.1.329", "message": {"content": content}});
            let marks = marks.as_object().unwrap().clone();
            line.as_object_mut().unwrap().extend(marks);
            line.to_string()
        };
        let typed = json!({"origin": {"kind": "human"}, "promptSource": "typed"});
        let prompt = line(typed, json!("go"));
        let over = json!({"type": "assistant", "timestamp": "2026-11-20T17:25:02.000Z",
            "sessionId": SESSION,
            "message": {"stop_reason": "end_turn", "content": [{"type": "text", "text": "OK"}]}})
        .to_string();
        let exit = "<command-name>/exit</command-name>\n<command-message>exit</command-message>";
        let asides = [
            line(
                json!({"isMeta": true}),
                json!("<local-command-caveat>Caveat"),
            ),
            line(
                json!({"isCompactSummary": true}),
                json!("This session is being continued"),
            ),
            line(
                json!({"origin": {"kind": "task-notification"}}),
                json!("<task-notification>done</task-notification>"),
            ),
            line(
                json!({"isMeta": true, "origin": {"kind": "peer"}}),
                json!([{"type": "text", "text": "report"}]),
            ),
            line(json!({}), json!(exit)),
            line(
                json!({}),
                json!("<local-command-stdout>Bye</local-command-stdout>"),
            ),
        ];
        let reads = |rows: &[&str]| says(waits(&rows.join("\n"), true, 0));
        for aside in &asides {
            // Under a reply that ended its turn, and under a prompt the model is at work on.
            assert_eq!(reads(&[&prompt, &over, aside]), "nothing", "{aside}");
            assert_eq!(
                reads(&[&over, &prompt, aside]),
                "open Some([2, 1, 329])",
                "{aside}"
            );
        }
    }

    #[test]
    fn a_receipt_and_an_event_count_only_in_their_own_calls_result() {
        let asks = "await tools.AskUserQuestion({questions: []})";
        let one = [repl("a", asks), answer("a", json!("(dispatched #1)"))];
        let call = |id: &str, name: &str| {
            let call = json!({"type": "tool_use", "id": id, "name": name, "input": {}});
            json!({"type": "assistant", "timestamp": "2026-11-20T17:25:02.000Z",
                "sessionId": SESSION, "message": {"content": [call]}})
            .to_string()
        };
        let reads = |rows: &[String]| says(waits(&rows.join("\n"), true, 0));
        let event = concat!(
            "<system>authentic event nonces: n</system>\n",
            r#"<event nonce="n" kind="repl-eval" at="t" eval="1" ms="1" status="settled">"#
        );
        // Poll's result ends the script. The same text as another tool's output, or as a
        // script's, ends nothing.
        let polled =
            |name: &str| [&one[..], &[call("b", name), answer("b", json!(event))]].concat();
        assert_eq!(reads(&polled("Poll")), "nothing");
        for name in ["Bash", "REPL"] {
            assert_eq!(reads(&polled(name)), "a script", "{name}");
        }
        // Nor is another tool's output a receipt: it takes no event from the script above it.
        let printed = [
            call("b", "Bash"),
            answer("b", json!("(dispatched #1)")),
            ended(1),
        ];
        assert_eq!(reads(&[&one[..], &printed[..]].concat()), "nothing");
        // A result whose call is above the read: what its text is, is not known.
        let unknown = "transcript unreadable or incomplete";
        for content in [event, "(dispatched #1)"] {
            assert_eq!(reads(&[answer("x", json!(content))]), unknown, "{content}");
        }
    }
}
