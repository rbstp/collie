use std::path::PathBuf;

use serde_json::Value;

use super::{Kind, TAIL_BYTES, Transcripts, open, tail, timestamp_ms};
use crate::prompt::Question;
use crate::runtime::AgentInfo;

/// The transcript of a terminal's Claude Code session and what ties a call in it to that
/// terminal's process, so the file is read without the lock on [`Transcripts`].
pub struct Asking {
    path: PathBuf,
    session: String,
    started_at: u64,
}

impl Transcripts {
    /// The session is the terminal's by process ([`Transcripts::holder`]).
    pub fn asking(&self, a: &AgentInfo) -> Option<Asking> {
        if a.agent.as_deref() != Some("claude") {
            return None;
        }
        let (session, started_at) = self.holder(a)?;
        Some(Asking {
            path: self.locate(Kind::Claude, &session, a)?,
            session,
            started_at,
        })
    }
}

impl Asking {
    /// The questions of the one call the session waits on, when that call is
    /// AskUserQuestion: what an approval matches a question form against
    /// ([`crate::prompt::Menu::asks`]). The call must be that session's and no older than
    /// its process, so one that a killed session left in a transcript another process
    /// resumed is none.
    pub fn questions(&self) -> Option<Vec<Question>> {
        let (file, len, _) = open(&self.path)?;
        let text = tail(file, len).ok()?;
        pending(&text, len <= TAIL_BYTES, &self.session, self.started_at)
    }
}

/// Read from the transcript's end up to the last assistant message's start: an assistant line
/// of another message, or, when the lines carry no message id, a `user` line that is not only
/// tool results. Claude Code writes one line per content block, so parallel calls are
/// several lines of one message, and a result or another line can come between them. Exactly
/// one call may be without a result: with a second one, the screen does not say which dialog
/// is drawn. A `user` line that is no result under the last call ends the turn: nothing is
/// pending. `whole` says the text starts at the file's start; otherwise a read that ends
/// before the message's start proves nothing about the calls above it and gives none.
fn pending(text: &str, whole: bool, session: &str, since: u64) -> Option<Vec<Question>> {
    let mut results = Vec::new();
    let mut calls = Vec::new();
    let mut message = None;
    let mut bounded = whole;
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
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
                results.extend(
                    blocks
                        .iter()
                        .filter(result)
                        .map(|b| b["tool_use_id"].clone()),
                );
            }
            Some("assistant") => {
                let id = &v["message"]["id"];
                if *message.get_or_insert_with(|| id.clone()) != *id {
                    bounded = true;
                    break;
                }
                for call in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    if !results.contains(&call["id"]) {
                        calls.push((call.clone(), v["sessionId"].clone(), v["timestamp"].clone()));
                    }
                }
            }
            _ => {}
        }
    }
    let [(call, by, at)] = &calls[..] else {
        return None;
    };
    let fresh = at
        .as_str()
        .and_then(timestamp_ms)
        .is_some_and(|at| at >= since);
    if !bounded || call["name"] != "AskUserQuestion" || by != session || !fresh {
        return None;
    }
    let questions = call["input"]["questions"].as_array()?;
    let questions: Option<Vec<Question>> = questions
        .iter()
        .map(|q| {
            let options = q["options"].as_array()?.iter().map(|o| {
                let description = o["description"].as_str().unwrap_or_default();
                Some((o["label"].as_str()?.to_owned(), description.to_owned()))
            });
            Some(Question {
                text: q["question"].as_str()?.to_owned(),
                options: options.collect::<Option<_>>()?,
                multi: q["multiSelect"] == true,
            })
        })
        .collect();
    questions.filter(|q| !q.is_empty())
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
    const SESSION: &str = "00000000-0000-4000-8000-000000000000";

    fn asked(transcript: &str) -> Option<Vec<Question>> {
        pending(transcript, true, SESSION, 0)
    }

    /// The options of a screen that `Approvals::screen` answers beside a pending call.
    fn offered(screen: &str, asked: &[Question]) -> Option<Vec<String>> {
        Menu::parse(screen)
            .filter(|m| m.on_screen() && m.asks(asked).is_some())
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
            assert_eq!(asked(transcript), None, "{screen}");
            assert!(!Menu::parse(screen).unwrap().is_question(), "{screen}");
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
        call[0].text = text;
        assert!(offered(&drawn, &call[1..]).is_some(), "reads as the second");
        assert_eq!(offered(&drawn, &call), None);
        call[0].text = "Which cache backend comes first?".into();
        assert_eq!(offered(screen, &call), None, "drawn whole too");
    }

    #[test]
    fn only_the_last_turns_one_call_without_a_result_is_pending() {
        let at = "2026-11-20T11:35:11.762Z";
        let then = timestamp_ms(at).unwrap();
        assert_eq!(pending(ONE.1, true, SESSION, then).unwrap().len(), 1);
        assert_eq!(
            pending(ONE.1, true, SESSION, then + 1),
            None,
            "older than the process"
        );
        let other = "11111111-0000-4000-8000-000000000000";
        assert_eq!(pending(ONE.1, true, other, 0), None, "another session's");
        // The first call was declined, a prompt followed, and the second call is pending.
        let rows: Vec<&str> = FIRST.1.lines().collect();
        let upto = |n: usize| asked(&rows[..n].join("\n")).map(|q| q.len());
        assert_eq!(
            [upto(2), upto(3), upto(4), upto(5), upto(6)],
            [Some(1), None, None, None, Some(2)]
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
        let asks =
            |rows: &[String], whole| pending(&rows.join("\n"), whole, SESSION, 0).map(|q| q.len());
        assert_eq!(
            asks(&[prompt(), ask("a", "m1"), side.to_string()], true),
            Some(1)
        );
        assert_eq!(
            asks(&[prompt(), call("a", "m1", "ExitPlanMode")], true),
            None
        );
        assert_eq!(
            asks(&[ask("a", "m1"), prompt()], true),
            None,
            "a prompt since"
        );
        assert_eq!(
            asks(&[prompt(), ask("a", "m1"), prompt(), ask("c", "m1")], true),
            None,
            "two questions pending, a line that is no result between them"
        );
        assert_eq!(
            asks(&[prompt(), call("b", "m1", "Bash"), ask("a", "m1")], true),
            None,
            "a Bash call pending beside it"
        );
        let read = [call("b", "m1", "Read"), result("b")];
        assert_eq!(
            asks(
                &[&[prompt(), ask("a", "m1")], &read[..], &[ask("c", "m1")]].concat(),
                true
            ),
            None,
            "two questions pending, a result between them"
        );
        assert_eq!(
            asks(&[&[prompt()], &read[..], &[ask("a", "m1")]].concat(), true),
            Some(1)
        );
        // A read that starts inside the turn: bounded by another message's line, or not.
        let earlier = [call("b", "m0", "Read"), result("b")];
        assert_eq!(
            asks(&[&earlier[..], &[ask("a", "m1")]].concat(), false),
            Some(1)
        );
        assert_eq!(asks(&[&read[..], &[ask("a", "m1")]].concat(), false), None);
        assert_eq!(
            asks(&[&read[..], &[ask("a", "m1")]].concat(), true),
            Some(1)
        );
    }
}
