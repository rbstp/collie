use std::path::PathBuf;

use serde_json::Value;

use super::{Kind, TAIL_BYTES, Transcripts, open, tail, timestamp_ms};
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
    /// One call, a Code Mode script that names AskUserQuestion: its whole string literals,
    /// which a question form is matched against ([`crate::prompt::Menu::scripted`]).
    Script {
        session: String,
        call: String,
        literals: Vec<String>,
    },
    /// One call that is no question.
    Other,
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

/// Read from the transcript's end up to the last assistant message's start: an assistant line
/// of another message, or, when the lines carry no message id, a `user` line that is not only
/// tool results. Claude Code writes one line per content block, so parallel calls are
/// several lines of one message, and a result or another line can come between them. With
/// more than one call without a result, the screen does not say which dialog is drawn. A
/// `user` line that is no result under the last call ends the turn: nothing is pending.
///
/// Nothing is concluded from a read that is not whole. A last line without its newline is
/// one Claude Code is still writing, and a line that does not parse may be cut: either can
/// be the result of the call above it, or a second call. `whole` says the text starts at the
/// file's start; otherwise a read that ends before the message's start says nothing of the
/// calls above it. The one call must be the session's and no older than its process
/// (`since`), so one that a killed session left in a transcript another process resumed is
/// not this terminal's.
fn pending(text: &str, whole: bool, session: &str, since: u64) -> Waits {
    let unknown = Waits::Unknown(Refused::Transcript);
    if !text.is_empty() && !text.ends_with('\n') {
        return unknown;
    }
    let mut results = Vec::new();
    let mut made = Vec::new();
    let mut calls = Vec::new();
    let mut message = None;
    let mut bounded = whole;
    for line in text.lines().rev().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return unknown;
        };
        if v["isSidechain"] == true {
            continue;
        }
        let blocks = v["message"]["content"].as_array();
        let blocks = blocks.map(Vec::as_slice).unwrap_or_default();
        match v["type"].as_str() {
            Some("user") => {
                let result = |b: &&Value| b["type"] == "tool_result";
                if (blocks.is_empty() || !blocks.iter().all(|b| result(&b)))
                    && !matches!(message, Some(Value::String(_)))
                {
                    bounded = true;
                    break;
                }
                let ids = blocks.iter().filter(result).map(|b| &b["tool_use_id"]);
                // A result above its own call says the file is not what Claude Code wrote.
                if ids.clone().any(|id| made.contains(id)) {
                    return unknown;
                }
                results.extend(ids.cloned());
            }
            Some("assistant") => {
                let id = &v["message"]["id"];
                if *message.get_or_insert_with(|| id.clone()) != *id {
                    bounded = true;
                    break;
                }
                for call in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    made.push(call["id"].clone());
                    if !results.contains(&call["id"]) {
                        calls.push((call.clone(), v["sessionId"].clone(), v["timestamp"].clone()));
                    }
                }
            }
            _ => {}
        }
    }
    let (call, by, at) = match &calls[..] {
        _ if !bounded => return unknown,
        [] => return Waits::Nothing,
        [one] => one,
        _ => return Waits::Unknown(Refused::Several),
    };
    let fresh = at
        .as_str()
        .and_then(timestamp_ms)
        .is_some_and(|at| at >= since);
    if by != session || !fresh {
        return Waits::Unknown(Refused::Stale);
    }
    // In Code Mode (2.1.329) the model's one tool is a script runner, and the question is a
    // function the script calls: the pending call is the script, and the transcript holds
    // nothing of the question but the script's text.
    if let (Some(id), Some(code)) = (call["id"].as_str(), call["input"]["code"].as_str())
        && call["name"] == "REPL"
        && code.contains("AskUserQuestion")
    {
        return Waits::Script {
            session: session.to_owned(),
            call: id.to_owned(),
            literals: literals(code),
        };
    }
    if call["name"] != "AskUserQuestion" {
        return Waits::Other;
    }
    let questions = call["input"]["questions"].as_array().and_then(|questions| {
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
    match (call["id"].as_str(), questions) {
        (Some(id), Some(questions)) if !questions.is_empty() => Waits::Question {
            session: session.to_owned(),
            call: id.to_owned(),
            questions,
        },
        _ => unknown,
    }
}

/// The string literals of a script, each the text between two matching quote characters
/// (`'`, `"` or a backtick) with `\n`, an escaped quote or backslash and `\u` escapes
/// undone. The script is not parsed: a quote in a comment or a regular expression opens a
/// literal like any other, and what follows can be read out of step, which loses literals
/// and adds none that a script could not hold outright. A literal with any other escape, a
/// template that holds `${`, and one a line break ends (which also brings the read back in
/// step after a stray apostrophe) are left out: none can be compared as it is written.
fn literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = code.chars().peekable();
    while let Some(quote) = chars.next() {
        if !matches!(quote, '\'' | '"' | '`') {
            continue;
        }
        let mut text = Some(String::new());
        let closed = loop {
            let undone = match chars.next() {
                None => break false,
                Some('\n') if quote != '`' => break false,
                Some(c) if c == quote => break true,
                Some('$') if quote == '`' && chars.peek() == Some(&'{') => None,
                Some('\\') => match chars.next() {
                    Some('n') => Some('\n'),
                    Some(c @ ('\'' | '"' | '`' | '\\')) => Some(c),
                    Some('u') => {
                        let digits: String = match chars.next_if_eq(&'{') {
                            Some(_) => chars.by_ref().take_while(|c| *c != '}').collect(),
                            None => chars.by_ref().take(4).collect(),
                        };
                        u32::from_str_radix(&digits, 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ => None,
                },
                c => c,
            };
            match (undone, &mut text) {
                (Some(c), Some(text)) => text.push(c),
                _ => text = None,
            }
        };
        out.extend(text.filter(|_| closed));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::prompt::{Menu, numbered};

    // Claude Code 2.1.329 in a terminal of 100 columns, captured 2026-11-20 (the plan file's
    // folder renamed): each screen with the last lines of its session's transcript at that
    // moment, reduced to the fields read here and its tool ids replaced. The pending call is
    // each transcript's last line. A question; the first question of a call with two, its
    // second, a multi-select one, the same with a box checked, and the call's review page; a
    // question of three lines, the second a row of `─`, drawn behind the gutter; a question
    // with four options in 12 rows, which scrolls, its cursor on option 1 and on option 4; a
    // Bash prompt and the plan prompt.
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
    const ONE: (&str, &str) = capture!("question-one");
    const FIRST: (&str, &str) = capture!("two-questions-first");
    const MULTI: (&str, &str) = capture!("two-questions-second-multiselect");
    const CHECKED: (&str, &str) = capture!("two-questions-second-checked");
    const REVIEW: (&str, &str) = capture!("two-questions-after-tab");
    const MULTILINE: (&str, &str) = capture!("question-multiline");
    const TALL: (&str, &str) = capture!("question-tall-12-rows");
    const TALL_ON_4: (&str, &str) = capture!("question-tall-12-rows-cursor-4");
    const BASH: (&str, &str) = capture!("bash");
    const PLAN: (&str, &str) = capture!("plan");
    const FORMS: [(&str, &str); 6] = [ONE, FIRST, MULTI, CHECKED, REVIEW, MULTILINE];
    // The same Claude Code with Code Mode on (`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1`): the
    // model's one tool is `REPL`, and the question is a function its script calls. The form
    // is drawn as outside Code Mode. A question written in the call, and one whose text and
    // labels the script puts in variables first.
    const SCRIPTED: (&str, &str) = capture!("codemode-question-one");
    const SCRIPTED_BUILT: (&str, &str) = capture!("codemode-question-built");
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
            Waits::Script { literals, .. } => format!("a script, {} literals", literals.len()),
            Waits::Other => "another call".to_owned(),
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
        // The first call was declined, a prompt followed, and the second call is pending.
        let rows: Vec<&str> = FIRST.1.lines().collect();
        let upto = |n: usize| says(waits(&rows[..n].join("\n"), true, 0));
        assert_eq!(
            [upto(2), upto(3), upto(4), upto(5), upto(6)],
            ["1 asked", "nothing", "nothing", "nothing", "2 asked"]
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
        let result = |id: &str| {
            let block = json!({"type": "tool_result", "tool_use_id": id});
            json!({"type": "user", "message": {"content": [block]}}).to_string()
        };
        let prompt = || json!({"type": "user", "message": {"content": "go"}}).to_string();
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
            "nothing",
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
        for (next, whole) in [(result("a"), "nothing"), (ask("c", "m1"), several)] {
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
        menu.scripted(&literals(code)).map(|_| menu.heads)
    }

    #[test]
    fn a_scripts_literals_are_read_whole_or_not_at_all() {
        let code = r#"f('x', "y\n\"z", `t`, `u${v}`, 'w\q', 'it\'s', '\u00e9\u{1F600}') // don't
'k' + 'un"#;
        assert_eq!(literals(code), ["x", "y\n\"z", "t", "it's", "é😀", "k"]);
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
            "`Which option do you want to pick for this ${'test'}?`",
            "'Which option do you want to pick for this\\x20test?'",
        ] {
            assert_ne!(code.replace(asks, written), code);
            assert_eq!(scripts(screen, &code.replace(asks, written)), not_found);
        }
        let built = code.replace("'Option A'", "'Option ' + 'A'");
        assert_ne!(built, code);
        assert_eq!(scripts(screen, &built), not_found);
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
        assert!(literals(&long).contains(&text));
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
    }
}
