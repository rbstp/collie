//! Token use and estimated API-equivalent cost from Claude Code, Codex and Copilot CLI
//! transcripts. Only token counts, model ids and 15-minute slots are kept; no message text.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Instant, UNIX_EPOCH};

use protocol::{CostRange, CostTokens, DayCost, ModelCost, UsageCosts};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::lock;
use crate::transcript::{civil, open, timestamp_ms};

pub const INDEX_FILE: &str = "usage-costs.json";
const INDEX_VERSION: u32 = 2;
const SLOT_MS: u64 = 15 * 60_000;
const DAY_MS: i64 = 86_400_000;
/// The longest range plus the widest phone offset: nothing older is ever shown.
const KEEP_MS: u64 = 32 * 86_400_000;
const CHUNK: usize = 1 << 20;
const MAX_LINE: usize = 8 << 20;
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 5;
const MAX_MODELS: usize = 32;
const MAX_MODEL_CHARS: usize = 64;
const RESCAN: std::time::Duration = std::time::Duration::from_secs(15);
/// OpenAI prices a call with more input than this, cached input included, at its long context rates.
const LONG_INPUT: u64 = 272_000;

/// LiteLLM's Claude and OpenAI chat prices, from `just prices`.
static PRICES: LazyLock<HashMap<String, Price>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/prices.json")).expect("bundled prices parse")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Agent {
    Claude,
    Codex,
    Copilot,
}

impl Agent {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
        }
    }
}

/// `input` excludes cached input; `cache_write_1h` is the part of `cache_write` held for an hour.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    #[serde(default)]
    cache_write_1h: u64,
}

impl Tokens {
    fn add(&mut self, o: &Tokens) {
        self.input = self.input.saturating_add(o.input);
        self.output = self.output.saturating_add(o.output);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.cache_write_1h = self.cache_write_1h.saturating_add(o.cache_write_1h);
    }

    fn is_empty(&self) -> bool {
        self.input == 0 && self.output == 0 && self.cache_read == 0 && self.cache_write == 0
    }

    fn wire(&self) -> CostTokens {
        CostTokens {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Price {
    input_cost_per_token: f64,
    output_cost_per_token: f64,
    cache_read_input_token_cost: Option<f64>,
    cache_creation_input_token_cost: Option<f64>,
    cache_creation_input_token_cost_above_1hr: Option<f64>,
    input_cost_per_token_above_272k_tokens: Option<f64>,
    output_cost_per_token_above_272k_tokens: Option<f64>,
    cache_read_input_token_cost_above_272k_tokens: Option<f64>,
    cache_creation_input_token_cost_above_272k_tokens: Option<f64>,
    fast: Option<f64>,
}

impl Price {
    /// Cost and what the cache reads saved against the input rate. Cached input the table
    /// does not price separately costs the input rate, never nothing.
    fn cost(&self, t: &Tokens, fast: bool, long: bool) -> (f64, f64) {
        let scale = self
            .fast
            .filter(|f| fast && f.is_finite() && *f > 0.0)
            .unwrap_or(1.0);
        let tier = |rate: Option<f64>| rate.filter(|_| long);
        let input =
            tier(self.input_cost_per_token_above_272k_tokens).unwrap_or(self.input_cost_per_token);
        let output = tier(self.output_cost_per_token_above_272k_tokens)
            .unwrap_or(self.output_cost_per_token);
        let read = tier(self.cache_read_input_token_cost_above_272k_tokens)
            .or(self.cache_read_input_token_cost)
            .unwrap_or(input);
        let write = tier(self.cache_creation_input_token_cost_above_272k_tokens)
            .or(self.cache_creation_input_token_cost)
            .unwrap_or(input);
        let write_1h = self
            .cache_creation_input_token_cost_above_1hr
            .unwrap_or(write);
        let hour = t.cache_write_1h.min(t.cache_write);
        let cost = t.input as f64 * input
            + t.output as f64 * output
            + t.cache_read as f64 * read
            + (t.cache_write - hour) as f64 * write
            + hour as f64 * write_1h;
        let saved = t.cache_read as f64 * (input - read).max(0.0);
        (cost * scale, saved * scale)
    }
}

/// Copilot writes `claude-opus-5.5` where Claude Code and the table write `claude-opus-5-5`,
/// and Claude Code adds `[1m]` for the long context tier.
fn price(model: &str) -> Option<&'static Price> {
    let mut id = model.trim().to_ascii_lowercase();
    if let Some(i) = id.find('[') {
        id.truncate(i);
    }
    if let Some(i) = id.rfind('/') {
        id.drain(..=i);
    }
    if id.starts_with("claude-") {
        id = id.replace('.', "-");
    }
    PRICES.get(&id).or_else(|| {
        let (base, date) = id.rsplit_once('-')?;
        (date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
            .then(|| PRICES.get(base))
            .flatten()
    })
}

fn model_id(model: Option<&str>) -> String {
    model
        .filter(|m| {
            !m.is_empty()
                && m.len() <= MAX_MODEL_CHARS
                && m.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:/[]-".contains(&b))
        })
        .unwrap_or("unknown")
        .to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Bucket {
    slot: u32,
    model: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    fast: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    long: bool,
    tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    agent: Agent,
    dev: u64,
    ino: u64,
    size: u64,
    /// Just past the last complete line read: a partial last line is read again.
    offset: u64,
    /// Codex: the latest `turn_context` model and the last counted `token_count`.
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    last: Option<u64>,
    buckets: Vec<Bucket>,
    /// Claude records this file counted first, by `message.id` and `requestId`, with their
    /// slot and the output counted so far.
    keys: Vec<(u64, u32, u64)>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    files: BTreeMap<String, Entry>,
}

#[derive(Default)]
struct State {
    loaded: bool,
    changed: bool,
    index: Index,
    seen: HashSet<u64>,
}

impl State {
    fn forget(&mut self, path: &str) {
        if let Some(e) = self.index.files.remove(path) {
            for (k, _, _) in e.keys {
                self.seen.remove(&k);
            }
            self.changed = true;
        }
    }

    /// The first file to count a Claude record keeps it, so a resumed session's copies and
    /// the repeats for each content block count once.
    fn merge(&mut self, path: &str, read: Delta, from_start: bool, cutoff: u32) {
        if from_start {
            self.forget(path);
        }
        let entry = self
            .index
            .files
            .entry(path.to_owned())
            .or_insert_with(|| Entry {
                agent: read.agent,
                dev: read.dev,
                ino: read.ino,
                size: 0,
                offset: 0,
                model: None,
                last: None,
                buckets: Vec::new(),
                keys: Vec::new(),
            });
        entry.dev = read.dev;
        entry.ino = read.ino;
        entry.size = read.size;
        entry.offset = read.offset;
        entry.model = read.parser.model;
        entry.last = read.parser.last;
        let mut at: HashMap<(u32, String, bool, bool), usize> = entry
            .buckets
            .iter()
            .enumerate()
            .map(|(i, b)| ((b.slot, b.model.clone(), b.fast, b.long), i))
            .collect();
        let mut owned: HashMap<u64, usize> = entry
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.0, i))
            .collect();
        for r in read.parser.records {
            let mut slot = (r.ms / SLOT_MS) as u32;
            if slot < cutoff {
                continue;
            }
            let mut tokens = r.tokens;
            if let Some(k) = r.key {
                if let Some(&i) = owned.get(&k) {
                    // A streamed message's earlier block records carry a partial output count.
                    let (_, first, counted) = entry.keys[i];
                    if tokens.output <= counted {
                        continue;
                    }
                    entry.keys[i].2 = tokens.output;
                    tokens = Tokens {
                        output: tokens.output - counted,
                        ..Tokens::default()
                    };
                    slot = first;
                } else if !self.seen.insert(k) {
                    continue;
                } else {
                    owned.insert(k, entry.keys.len());
                    entry.keys.push((k, slot, tokens.output));
                }
            }
            let i = *at
                .entry((slot, r.model.clone(), r.fast, r.long))
                .or_insert_with(|| {
                    entry.buckets.push(Bucket {
                        slot,
                        model: r.model,
                        fast: r.fast,
                        long: r.long,
                        tokens: Tokens::default(),
                    });
                    entry.buckets.len() - 1
                });
            entry.buckets[i].tokens.add(&tokens);
        }
        self.changed = true;
    }

    fn prune(&mut self, cutoff: u32) {
        for e in self.index.files.values_mut() {
            let (buckets, keys) = (e.buckets.len(), e.keys.len());
            e.buckets.retain(|b| b.slot >= cutoff);
            e.keys.retain(|(k, slot, _)| {
                *slot >= cutoff || {
                    self.seen.remove(k);
                    false
                }
            });
            self.changed |= buckets != e.buckets.len() || keys != e.keys.len();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub copilot: Option<PathBuf>,
}

impl Roots {
    /// Files modified since `keep`, and whether no root hit the file cap.
    fn files(&self, keep: u64) -> (Vec<(PathBuf, Agent)>, bool) {
        let mut out = Vec::new();
        let mut complete = true;
        let mut add = |root: &Option<PathBuf>, agent: Agent, wanted: fn(&str) -> bool| {
            let Some(root) = root else { return };
            let mut found = Vec::new();
            walk(root, 0, keep, wanted, &mut found);
            complete &= found.len() < MAX_FILES;
            out.extend(found.into_iter().map(|p| (p, agent)));
        };
        add(&self.claude, Agent::Claude, |n| n.ends_with(".jsonl"));
        add(&self.codex, Agent::Codex, |n| {
            n.starts_with("rollout-") && n.ends_with(".jsonl")
        });
        add(&self.copilot, Agent::Copilot, |n| n == "events.jsonl");
        out.sort();
        (out, complete)
    }
}

/// Never through a symlink or into a hidden folder; at most `MAX_FILES` per root.
fn walk(dir: &Path, depth: usize, keep: u64, wanted: fn(&str) -> bool, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = e.file_name();
        let Some(name) = name.to_str().filter(|n| !n.starts_with('.')) else {
            continue;
        };
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() && depth < MAX_DEPTH {
            walk(&e.path(), depth + 1, keep, wanted, out);
        } else if kind.is_file()
            && wanted(name)
            && e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .is_ok_and(|d| d.as_millis() as u64 >= keep)
                })
        {
            out.push(e.path());
        }
    }
}

struct Record {
    ms: u64,
    model: String,
    fast: bool,
    long: bool,
    tokens: Tokens,
    key: Option<u64>,
}

struct Parser {
    agent: Agent,
    model: Option<String>,
    last: Option<u64>,
    records: Vec<Record>,
}

#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation: Option<ClaudeCacheCreation>,
    speed: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeCacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
}

fn hash(text: &str) -> u64 {
    let digest = Sha256::digest(text.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"))
}

impl Parser {
    fn line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        let record = match self.agent {
            Agent::Claude if text.contains("\"usage\"") => claude(text),
            Agent::Codex
                if text.contains("\"token_count\"") || text.contains("\"turn_context\"") =>
            {
                self.codex(text)
            }
            Agent::Copilot if text.contains("\"session.usage_record\"") => copilot(text),
            _ => None,
        };
        if let Some(r) = record.filter(|r| !r.tokens.is_empty()) {
            self.records.push(r);
        }
    }

    /// `token_count` carries no model, so it comes from the latest `turn_context`. Codex
    /// repeats an unchanged event on some stream boundaries; the cumulative total tells them apart.
    fn codex(&mut self, text: &str) -> Option<Record> {
        let v: Value = serde_json::from_str(text).ok()?;
        let p = &v["payload"];
        if v["type"] == "turn_context" {
            if let Some(model) = p["model"].as_str() {
                self.model = Some(model_id(Some(model)));
            }
            return None;
        }
        if v["type"] != "event_msg" || p["type"] != "token_count" {
            return None;
        }
        let info = &p["info"];
        let last = info["last_token_usage"].as_object()?;
        let model = self.model.clone()?;
        let ms = timestamp_ms(v["timestamp"].as_str()?)?;
        let total = &info["total_token_usage"];
        let counted = if total.is_object() {
            total
        } else {
            &info["last_token_usage"]
        };
        let signature = hash(&counted.to_string());
        if self.last == Some(signature) {
            return None;
        }
        self.last = Some(signature);
        let n = |k: &str| last.get(k).and_then(Value::as_u64).unwrap_or(0);
        let (read, write) = (n("cached_input_tokens"), n("cache_write_input_tokens"));
        Some(Record {
            ms,
            model,
            fast: false,
            long: n("input_tokens") > LONG_INPUT,
            tokens: Tokens {
                input: n("input_tokens").saturating_sub(read).saturating_sub(write),
                output: n("output_tokens"),
                cache_read: read,
                cache_write: write,
                cache_write_1h: 0,
            },
            key: None,
        })
    }
}

/// Sidechain (subagent) records count; Claude Code's own `<synthetic>` notices cost nothing.
fn claude(text: &str) -> Option<Record> {
    let line: ClaudeLine = serde_json::from_str(text).ok()?;
    if line.kind.as_deref() != Some("assistant") {
        return None;
    }
    let message = line.message?;
    let usage = message.usage?;
    if message.model.as_deref() == Some("<synthetic>") {
        return None;
    }
    let ms = timestamp_ms(line.timestamp.as_deref()?)?;
    let key = (message.id.is_some() || line.request_id.is_some()).then(|| {
        hash(&format!(
            "{}:{}",
            message.id.as_deref().unwrap_or(""),
            line.request_id.as_deref().unwrap_or("")
        ))
    });
    Some(Record {
        ms,
        model: model_id(message.model.as_deref()),
        fast: usage.speed.as_deref() == Some("fast"),
        long: false,
        tokens: Tokens {
            input: usage.input_tokens.unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
            cache_read: usage.cache_read_input_tokens.unwrap_or(0),
            cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
            cache_write_1h: usage
                .cache_creation
                .and_then(|c| c.ephemeral_1h_input_tokens)
                .unwrap_or(0),
        },
        key,
    })
}

/// One record per model call, subagents' included; `inputTokens` includes the cached input.
fn copilot(text: &str) -> Option<Record> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v["type"] != "session.usage_record" {
        return None;
    }
    let u = &v["data"]["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    let (read, write) = (n("cacheReadTokens"), n("cacheWriteTokens"));
    Some(Record {
        ms: timestamp_ms(v["timestamp"].as_str()?)?,
        model: model_id(u["model"].as_str()),
        fast: false,
        long: n("inputTokens") > LONG_INPUT,
        tokens: Tokens {
            input: n("inputTokens").saturating_sub(read).saturating_sub(write),
            output: n("outputTokens"),
            cache_read: read,
            cache_write: write,
            cache_write_1h: 0,
        },
        key: None,
    })
}

struct Delta {
    agent: Agent,
    dev: u64,
    ino: u64,
    size: u64,
    offset: u64,
    parser: Parser,
}

/// Reads `start..size` a chunk at a time and consumes only newline-terminated lines.
fn read_lines(
    mut file: File,
    start: u64,
    size: u64,
    mut parser: Parser,
) -> std::io::Result<(u64, Parser)> {
    file.seek(SeekFrom::Start(start))?;
    let mut file = file.take(size - start);
    let mut buf = vec![0; CHUNK];
    let mut line = Vec::new();
    let mut skipping = false;
    let (mut pos, mut offset) = (start, start);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok((offset, parser));
        }
        let mut chunk = &buf[..n];
        while let Some(i) = chunk.iter().position(|&b| b == b'\n') {
            if !skipping {
                line.extend_from_slice(&chunk[..i]);
                parser.line(&line);
            }
            line.clear();
            skipping = false;
            pos += i as u64 + 1;
            offset = pos;
            chunk = &chunk[i + 1..];
        }
        pos += chunk.len() as u64;
        if skipping || line.len() + chunk.len() > MAX_LINE {
            line.clear();
            skipping = true;
        } else {
            line.extend_from_slice(chunk);
        }
    }
}

pub struct Costs {
    path: PathBuf,
    roots: Roots,
    state: Mutex<State>,
    scanning: AtomicBool,
    finished: Mutex<Option<Instant>>,
}

struct Scanned<'a>(&'a Costs);

impl Drop for Scanned<'_> {
    fn drop(&mut self) {
        *lock(&self.0.finished) = Some(Instant::now());
        self.0.scanning.store(false, Ordering::SeqCst);
    }
}

impl Costs {
    pub fn new(path: PathBuf, roots: Roots) -> Self {
        Self {
            path,
            roots,
            state: Mutex::new(State::default()),
            scanning: AtomicBool::new(false),
            finished: Mutex::new(None),
        }
    }

    /// Starts a scan on its own thread unless one runs or one finished moments ago; never waits.
    pub fn refresh(self: &Arc<Self>) {
        if lock(&self.finished).is_some_and(|t| t.elapsed() < RESCAN)
            || self.scanning.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let costs = self.clone();
        let spawned = std::thread::Builder::new()
            .name("usage-costs".into())
            .spawn(move || {
                let _done = Scanned(&costs);
                costs.scan(crate::now_ms());
            });
        if spawned.is_err() {
            self.scanning.store(false, Ordering::SeqCst);
        }
    }

    fn load(&self) {
        if lock(&self.state).loaded {
            return;
        }
        let index = crate::peers::load_json::<Index>(&self.path)
            .ok()
            .filter(|i| i.version == INDEX_VERSION)
            .unwrap_or(Index {
                version: INDEX_VERSION,
                files: BTreeMap::new(),
            });
        let seen = index
            .files
            .values()
            .flat_map(|e| e.keys.iter().map(|k| k.0))
            .collect();
        *lock(&self.state) = State {
            loaded: true,
            changed: false,
            index,
            seen,
        };
    }

    /// The state lock is held only to merge one file, so a summary sees the totals so far.
    fn scan(&self, now: u64) {
        self.load();
        let keep = now.saturating_sub(KEEP_MS);
        let cutoff = (keep / SLOT_MS) as u32;
        let (files, complete) = self.roots.files(keep);
        if complete {
            let listed: HashSet<&str> = files.iter().filter_map(|(p, _)| p.to_str()).collect();
            let mut st = lock(&self.state);
            let gone: Vec<String> = st
                .index
                .files
                .keys()
                .filter(|p| !listed.contains(p.as_str()))
                .cloned()
                .collect();
            for p in gone {
                st.forget(&p);
            }
        }
        for (path, agent) in files {
            let Some(key) = path.to_str() else { continue };
            let opened = open(&path).and_then(|(file, size, modified)| {
                let meta = file.metadata().ok()?;
                let ms = modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
                Some((file, size, ms, meta.dev(), meta.ino()))
            });
            let Some((file, size, _, dev, ino)) = opened.filter(|o| o.2 >= keep) else {
                lock(&self.state).forget(key);
                continue;
            };
            let previous = lock(&self.state).index.files.get(key).map(|e| {
                (e.dev == dev && e.ino == ino && size >= e.size && size >= e.offset)
                    .then(|| (e.size, e.offset, e.model.clone(), e.last))
            });
            let resume = previous.flatten();
            if resume.as_ref().is_some_and(|r| r.0 == size) {
                continue;
            }
            let (start, model, last) = resume.map_or((0, None, None), |r| (r.1, r.2, r.3));
            let parser = Parser {
                agent,
                model,
                last,
                records: Vec::new(),
            };
            let Ok((offset, parser)) = read_lines(file, start, size, parser) else {
                continue;
            };
            let read = Delta {
                agent,
                dev,
                ino,
                size,
                offset,
                parser,
            };
            lock(&self.state).merge(key, read, start == 0, cutoff);
        }
        let bytes = {
            let mut st = lock(&self.state);
            st.prune(cutoff);
            if !st.changed {
                return;
            }
            st.changed = false;
            serde_json::to_vec(&st.index)
        };
        let saved = bytes.map_err(anyhow::Error::from).and_then(|bytes| {
            let tmp = self.path.with_extension("json.tmp");
            crate::replace_file(&self.path, &tmp, &bytes, 0o600)
        });
        if let Err(e) = saved {
            lock(&self.state).changed = true;
            tracing::warn!(error = %e, "could not save the usage cost index");
        }
    }

    pub fn summary(&self, range: CostRange, utc_offset_minutes: i16, now: u64) -> UsageCosts {
        let offset = i64::from(utc_offset_minutes) * 60_000;
        let day = |ms: u64| (ms as i64 + offset).div_euclid(DAY_MS);
        let days = i64::from(range.days());
        let today = day(now);
        let first = today - days + 1;
        let mut daily = vec![0.0; days as usize];
        let mut models: BTreeMap<(Agent, &str), (Tokens, f64, f64)> = BTreeMap::new();
        let st = lock(&self.state);
        for e in st.index.files.values() {
            for b in &e.buckets {
                let d = day(u64::from(b.slot) * SLOT_MS);
                if !(first..=today).contains(&d) {
                    continue;
                }
                let (tokens, cost, saved) = models.entry((e.agent, &b.model)).or_default();
                tokens.add(&b.tokens);
                if let Some(price) = price(&b.model) {
                    let (c, s) = price.cost(&b.tokens, b.fast, b.long);
                    *cost += c;
                    *saved += s;
                    daily[(d - first) as usize] += c;
                }
            }
        }
        let mut tokens = Tokens::default();
        let (mut cost_usd, mut cache_savings_usd) = (0.0, 0.0);
        let mut rows: Vec<ModelCost> = models
            .into_iter()
            .map(|((agent, model), (t, cost, saved))| {
                tokens.add(&t);
                cost_usd += cost;
                cache_savings_usd += saved;
                let priced = price(model).is_some();
                ModelCost {
                    agent: agent.name().to_owned(),
                    model: model.to_owned(),
                    tokens: t.wire(),
                    cost_usd: priced.then_some(cost),
                    cache_savings_usd: priced.then_some(saved),
                }
            })
            .collect();
        let sum = |t: &CostTokens| {
            t.input
                .saturating_add(t.output)
                .saturating_add(t.cache_read)
                .saturating_add(t.cache_write)
        };
        rows.sort_by(|a, b| {
            b.cost_usd
                .unwrap_or(-1.0)
                .total_cmp(&a.cost_usd.unwrap_or(-1.0))
                .then(sum(&b.tokens).cmp(&sum(&a.tokens)))
        });
        rows.truncate(MAX_MODELS);
        UsageCosts {
            scanning: self.scanning.load(Ordering::SeqCst),
            tokens: tokens.wire(),
            cost_usd,
            cache_savings_usd,
            days: daily
                .into_iter()
                .enumerate()
                .map(|(i, cost_usd)| {
                    let (y, m, d) = civil(first + i as i64);
                    DayCost {
                        date: format!("{y:04}-{m:02}-{d:02}"),
                        cost_usd,
                    }
                })
                .collect(),
            models: rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const CLAUDE: &str = include_str!("../tests/fixtures/transcripts/claude-costs.jsonl");
    const RESUMED: &str = include_str!("../tests/fixtures/transcripts/claude-costs-resumed.jsonl");
    const CODEX: &str = include_str!("../tests/fixtures/transcripts/codex-costs.jsonl");
    const COPILOT: &str = include_str!("../tests/fixtures/transcripts/copilot-costs.jsonl");
    const SUBAGENT: &str =
        include_str!("../tests/fixtures/transcripts/claude-costs-subagent.jsonl");
    const COPILOT_SUBAGENTS: &str =
        include_str!("../tests/fixtures/transcripts/copilot-subagents.jsonl");

    struct Setup {
        _dir: tempfile::TempDir,
        project: PathBuf,
        codex: PathBuf,
        index: PathBuf,
        roots: Roots,
        costs: Arc<Costs>,
    }

    impl Setup {
        fn original(&self) -> PathBuf {
            self.project
                .join("10000000-0000-4000-8000-0000000000c1.jsonl")
        }

        fn resumed(&self) -> PathBuf {
            self.project
                .join("10000000-0000-4000-8000-0000000000c2.jsonl")
        }

        fn scan(&self) -> UsageCosts {
            self.costs.scan(now());
            self.costs.summary(CostRange::Month, 0, now())
        }
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let project = root.join("claude/projects/-work-app");
        let day = root.join("codex/sessions/2026/10/18");
        let session = root.join("copilot/session-state/10000000-0000-4000-8000-0000000000d1");
        let data = root.join("data");
        for d in [&project, &day, &session, &data] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        let codex =
            day.join("rollout-2026-10-18T07-59-00-01a10f07-b5ed-79a3-989d-fa10b96c5c01.jsonl");
        std::fs::write(&codex, CODEX).unwrap();
        std::fs::write(session.join("events.jsonl"), COPILOT).unwrap();
        let roots = Roots {
            claude: Some(root.join("claude/projects")),
            codex: Some(root.join("codex/sessions")),
            copilot: Some(root.join("copilot/session-state")),
        };
        let index = data.join(INDEX_FILE);
        let setup = Setup {
            costs: Arc::new(Costs::new(index.clone(), roots.clone())),
            _dir: dir,
            project,
            codex,
            index,
            roots,
        };
        std::fs::write(setup.original(), CLAUDE).unwrap();
        std::fs::write(setup.resumed(), RESUMED).unwrap();
        setup
    }

    fn now() -> u64 {
        timestamp_ms("2026-10-18T20:00:00.000Z").unwrap()
    }

    fn row<'a>(c: &'a UsageCosts, agent: &str, model: &str) -> &'a ModelCost {
        c.models
            .iter()
            .find(|m| m.agent == agent && m.model == model)
            .unwrap_or_else(|| panic!("no {agent} {model}"))
    }

    fn tokens(input: u64, output: u64, cache_read: u64, cache_write: u64) -> CostTokens {
        CostTokens {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    fn append(path: &Path, text: &str) {
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn claude_record(id: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","model":"claude-opus-5-5","content":[],"usage":{{"input_tokens":{input},"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":{output}}}}},"requestId":"req_{id}","timestamp":"2026-10-18T15:00:00.000Z"}}"#
        )
    }

    #[test]
    fn counts_each_agent_once_and_prices_it() {
        let s = setup();
        let c = s.scan();
        assert!(!c.scanning);
        let opus = row(&c, "claude", "claude-opus-5-5");
        // msg_cost_a once (two blocks, then a resumed copy), msg_cost_b in fast mode once, msg_cost_e.
        assert_eq!(opus.tokens, tokens(6, 900, 71_000, 1_200));
        assert!(close(opus.cost_usd, 0.022012 + 0.022416 + 0.008004));
        assert!(close(opus.cache_savings_usd, 0.076 + 0.1596 + 0.114));
        let haiku = row(&c, "claude", "claude-haiku-4-5-20251001");
        assert_eq!(haiku.tokens, tokens(100, 50, 0, 0));
        assert!(close(haiku.cost_usd, 0.00035));
        let unknown = row(&c, "claude", "claude-unreleased-9");
        assert_eq!(unknown.tokens, tokens(10, 5, 0, 0));
        assert_eq!((unknown.cost_usd, unknown.cache_savings_usd), (None, None));
        assert!(!c.models.iter().any(|m| m.model.contains("synthetic")));

        let sol = row(&c, "codex", "gpt-6.1-sol");
        assert_eq!(sol.tokens, tokens(1_100, 80, 1_400, 0));
        assert!(close(sol.cost_usd, 0.00314));
        assert!(close(sol.cache_savings_usd, 0.00266));
        let astra = row(&c, "codex", "gpt-6-astra");
        assert_eq!(astra.tokens, tokens(1_000, 20, 0, 0));
        assert!(close(astra.cost_usd, 0.011));

        let copilot_opus = row(&c, "copilot", "claude-opus-5.5");
        assert_eq!(copilot_opus.tokens, tokens(100, 200, 4_000, 900));
        assert!(close(copilot_opus.cost_usd, 0.0097));
        let luna = row(&c, "copilot", "gpt-5.6-luna");
        assert_eq!(luna.tokens, tokens(20, 150, 2_990, 3_490));
        assert!(close(luna.cost_usd, 0.0011163));

        assert_eq!(c.models.len(), 7);
        assert_eq!(c.models[0].model, "claude-opus-5-5");
        let priced: f64 = c.models.iter().filter_map(|m| m.cost_usd).sum();
        assert!(close(Some(c.cost_usd), priced));
        assert_eq!(c.tokens.input, 6 + 100 + 10 + 1_100 + 1_000 + 100 + 20);
        assert_eq!(c.days.len(), 30);
        assert_eq!(c.days[29].date, "2026-10-18");
        assert_eq!(c.days[28].date, "2026-10-17");
        assert_eq!(c.days[0].date, "2026-09-19");
        assert!(close(Some(c.days[28].cost_usd), 0.00035));
        assert!(close(Some(c.days[29].cost_usd), priced - 0.00035));
        assert!(c.days[..28].iter().all(|d| d.cost_usd == 0.0));
    }

    #[test]
    fn days_end_at_the_phone_midnight() {
        let s = setup();
        s.costs.scan(now());
        let day = |offset| {
            let c = s.costs.summary(CostRange::Today, offset, now());
            assert_eq!(c.days.len(), 1);
            (
                c.days[0].date.clone(),
                c.models.iter().any(|m| m.model.starts_with("claude-haiku")),
            )
        };
        assert_eq!(day(0), ("2026-10-18".to_owned(), false));
        assert_eq!(day(30), ("2026-10-18".to_owned(), true));
        assert_eq!(day(-720), ("2026-10-18".to_owned(), false));
        assert_eq!(day(300), ("2026-10-19".to_owned(), false));
        let week = s.costs.summary(CostRange::Week, -720, now());
        assert_eq!(week.days.len(), 7);
        assert_eq!(week.days[6].date, "2026-10-18");
        assert!(close(Some(week.days[5].cost_usd), 0.00035));
    }

    #[test]
    fn a_second_scan_reads_only_new_bytes_and_a_partial_line_counts_once() {
        let s = setup();
        let before = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        // Blank the counted msg_cost_e in place: a scan from the start would lose it.
        let text = std::fs::read_to_string(s.resumed()).unwrap();
        let e = text.lines().nth(3).unwrap();
        std::fs::write(s.resumed(), text.replace(e, &" ".repeat(e.len()))).unwrap();
        append(
            &s.resumed(),
            &format!("{}\n", claude_record("msg_cost_f", 7, 70)),
        );
        let after = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        assert_eq!(after.input, before.input + 7);
        assert_eq!(after.output, before.output + 70);

        let partial = claude_record("msg_cost_g", 9, 90);
        append(&s.resumed(), &partial[..partial.len() - 1]);
        assert_eq!(row(&s.scan(), "claude", "claude-opus-5-5").tokens, after);
        let entry = |s: &Setup| {
            let st = lock(&s.costs.state);
            let e = &st.index.files[s.resumed().to_str().unwrap()];
            (e.offset, e.size)
        };
        let (offset, size) = entry(&s);
        assert!(offset < size);
        append(&s.resumed(), "}\n");
        let done = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        assert_eq!(done.input, after.input + 9);
        assert_eq!(row(&s.scan(), "claude", "claude-opus-5-5").tokens, done);
        let (offset, size) = entry(&s);
        assert_eq!(offset, size);
    }

    #[test]
    fn a_streamed_message_counts_its_final_output_once() {
        let s = setup();
        let before = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        let subagents = s
            .project
            .join("10000000-0000-4000-8000-0000000000c1/subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let agent = subagents.join("agent-a1b2c3d4e5f6a7b8c.jsonl");
        let (first, last) = SUBAGENT.split_once('\n').unwrap();
        std::fs::write(&agent, format!("{first}\n")).unwrap();
        let partial = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        assert_eq!(partial.output, before.output + 21);
        append(&agent, last);
        let done = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        assert_eq!(done.output, before.output + 234);
        assert_eq!(done.input, before.input + 2);
        assert_eq!(done.cache_read, before.cache_read + 75_954);

        std::fs::remove_file(&agent).unwrap();
        s.scan();
        std::fs::write(subagents.join("agent-b1b2c3d4e5f6a7b8c.jsonl"), SUBAGENT).unwrap();
        assert_eq!(row(&s.scan(), "claude", "claude-opus-5-5").tokens, done);
    }

    #[test]
    fn long_codex_and_copilot_calls_use_the_long_context_rates() {
        let s = setup();
        let base = row(&s.scan(), "codex", "gpt-6-astra").cost_usd.unwrap();
        append(
            &s.codex,
            r#"{"timestamp":"2026-10-18T12:20:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":303500,"cached_input_tokens":101400,"output_tokens":110,"total_tokens":303610},"last_token_usage":{"input_tokens":300000,"cached_input_tokens":100000,"cache_write_input_tokens":0,"output_tokens":10,"total_tokens":300010}}}}
"#,
        );
        let astra = price("gpt-6-astra").unwrap();
        let long = |rate: Option<f64>| rate.unwrap();
        let expected = 200_000.0 * long(astra.input_cost_per_token_above_272k_tokens)
            + 10.0 * long(astra.output_cost_per_token_above_272k_tokens)
            + 100_000.0 * long(astra.cache_read_input_token_cost_above_272k_tokens);
        let cost = row(&s.scan(), "codex", "gpt-6-astra").cost_usd.unwrap();
        assert!((cost - base - expected).abs() < 1e-9);
        let t = Tokens {
            input: 1_000,
            output: 1_000,
            cache_read: 1_000,
            cache_write: 0,
            cache_write_1h: 0,
        };
        assert!(astra.cost(&t, false, true).0 > astra.cost(&t, false, false).0);
        let opus = price("claude-opus-5-5").unwrap();
        assert_eq!(opus.cost(&t, false, true), opus.cost(&t, false, false));
        assert!(copilot(r#"{"type":"session.usage_record","timestamp":"2026-10-18T12:00:00.000Z","data":{"usage":{"model":"gpt-6.1-sol","inputTokens":272001,"outputTokens":1}}}"#).unwrap().long);
    }

    #[test]
    fn huge_counts_saturate() {
        let s = setup();
        append(
            &s.resumed(),
            &format!("{}\n", claude_record("msg_cost_big", u64::MAX, 1)),
        );
        let c = s.scan();
        assert_eq!(c.tokens.input, u64::MAX);
        assert_eq!(c.models[0].tokens.input, u64::MAX);
    }

    #[test]
    fn a_shrunk_or_replaced_file_is_read_again_from_the_start() {
        let s = setup();
        s.scan();
        let kept: String = CODEX.lines().take(4).map(|l| format!("{l}\n")).collect();
        std::fs::File::options()
            .write(true)
            .open(&s.codex)
            .unwrap()
            .set_len(kept.len() as u64)
            .unwrap();
        let c = s.scan();
        assert_eq!(
            row(&c, "codex", "gpt-6.1-sol").tokens,
            tokens(600, 50, 400, 0)
        );
        assert!(!c.models.iter().any(|m| m.model == "gpt-6-astra"));

        // A rewrite under a new inode: its own records go, the original's stay counted once.
        let lines: Vec<&str> = RESUMED.lines().collect();
        let tmp = s.project.join("rewrite.tmp");
        std::fs::write(
            &tmp,
            format!(
                "{}\n{}\n{}\n",
                lines[0],
                lines[1],
                claude_record("msg_cost_h", 4, 40)
            ),
        )
        .unwrap();
        std::fs::rename(&tmp, s.resumed()).unwrap();
        let opus = row(&s.scan(), "claude", "claude-opus-5-5").tokens.clone();
        assert_eq!(opus, tokens(3 + 2 + 4, 500 + 300 + 40, 41_000, 1_200));
    }

    #[test]
    fn the_index_survives_a_restart_and_keeps_no_text() {
        let s = setup();
        let c = s.scan();
        let meta = std::fs::metadata(&s.index).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let text = std::fs::read_to_string(&s.index).unwrap();
        for kept_out in ["scrubbed", "msg_cost", "req_cost", "/work/app", "probe"] {
            assert!(!text.contains(kept_out), "{kept_out} leaked into the index");
        }
        let again = Costs::new(s.index.clone(), s.roots.clone());
        again.load();
        assert_eq!(again.summary(CostRange::Month, 0, now()), c);
        again.scan(now());
        assert_eq!(again.summary(CostRange::Month, 0, now()), c);
        // Everything here is past the kept window a month later, and gone files are forgotten.
        again.scan(now() + KEEP_MS + SLOT_MS);
        let later = again.summary(CostRange::Month, 0, now() + KEEP_MS + SLOT_MS);
        assert!(later.models.is_empty());
        assert!(lock(&again.state).seen.is_empty());
        std::fs::remove_file(s.original()).unwrap();
        again.scan(now());
        assert!(
            !lock(&again.state)
                .index
                .files
                .contains_key(s.original().to_str().unwrap())
        );
    }

    #[test]
    fn a_request_never_waits_for_a_scan() {
        let s = setup();
        let held = lock(&s.costs.state);
        s.costs.refresh();
        assert!(s.costs.scanning.load(Ordering::SeqCst));
        drop(held);
        for _ in 0..1000 {
            if !s.costs.scanning.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!s.costs.scanning.load(Ordering::SeqCst));
        assert_eq!(lock(&s.costs.state).index.files.len(), 4);
        s.costs.refresh();
        assert!(!s.costs.scanning.load(Ordering::SeqCst));
    }

    #[test]
    fn skips_symlinks_hidden_folders_and_other_files() {
        let s = setup();
        std::os::unix::fs::symlink(s.original(), s.project.join("link.jsonl")).unwrap();
        let hidden = s.project.join(".hidden");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("copy.jsonl"), CLAUDE).unwrap();
        std::fs::write(s.project.join("notes.txt"), CLAUDE).unwrap();
        let files = s.roots.files(0).0;
        assert_eq!(files.len(), 4);
        assert!(
            files
                .iter()
                .all(|(p, _)| !p.to_str().unwrap().contains("link")
                    && !p.to_str().unwrap().contains(".hidden"))
        );
    }

    #[test]
    fn copilot_records_without_a_model_have_no_price() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir
            .path()
            .join("session-state/10000000-0000-4000-8000-000000000001");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("events.jsonl"), COPILOT_SUBAGENTS).unwrap();
        let costs = Costs::new(
            dir.path().join(INDEX_FILE),
            Roots {
                copilot: Some(dir.path().join("session-state")),
                ..Roots::default()
            },
        );
        costs.scan(now());
        let c = costs.summary(CostRange::Week, 0, now());
        assert_eq!(c.models.len(), 1);
        assert_eq!(c.models[0].model, "unknown");
        assert_eq!(c.models[0].tokens, tokens(6_638 + 6_700, 45, 0, 0));
        assert_eq!(c.models[0].cost_usd, None);
        assert_eq!(c.cost_usd, 0.0);
    }

    #[test]
    fn prices_normalize_model_ids_and_apply_fast_mode() {
        let opus = price("claude-opus-5-5").unwrap();
        for alias in [
            "claude-opus-5.5",
            "Claude-Opus-5-5",
            "claude-opus-5-5[1m]",
            "anthropic/claude-opus-5-5",
        ] {
            assert!(std::ptr::eq(price(alias).unwrap(), opus), "{alias}");
        }
        assert!(std::ptr::eq(
            price("claude-opus-4-5-20990101").unwrap(),
            price("claude-opus-4-5").unwrap()
        ));
        assert!(price("claude-haiku-4-5-20251001").is_some());
        assert!(price("gpt-6.1-sol").is_some());
        for unpriced in ["opus", "codex-auto-review", "unknown", "<synthetic>", ""] {
            assert!(price(unpriced).is_none(), "{unpriced}");
        }
        let t = Tokens {
            input: 1_000,
            output: 1_000,
            cache_read: 1_000,
            cache_write: 1_000,
            cache_write_1h: 0,
        };
        let (standard, saved) = opus.cost(&t, false, false);
        let (fast, fast_saved) = opus.cost(&t, true, false);
        assert!((fast - 2.0 * standard).abs() < 1e-12);
        assert!((fast_saved - 2.0 * saved).abs() < 1e-12);
        let sol = price("gpt-6.1-sol").unwrap();
        assert_eq!(sol.cost(&t, true, false), sol.cost(&t, false, false));
        let hour = Tokens {
            cache_write_1h: 1_000,
            ..t
        };
        assert!((opus.cost(&hour, false, false).0 - standard - 1_000.0 * 3e-6).abs() < 1e-12);
    }

    #[test]
    fn model_ids_are_plain() {
        assert_eq!(model_id(Some("gpt-6.1-sol")), "gpt-6.1-sol");
        assert_eq!(model_id(Some("claude-opus-5-5[1m]")), "claude-opus-5-5[1m]");
        for bad in [
            None,
            Some(""),
            Some("a b"),
            Some("model\nname"),
            Some(&*"x".repeat(65)),
        ] {
            assert_eq!(model_id(bad), "unknown");
        }
    }
}
