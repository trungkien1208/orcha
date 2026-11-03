//! orcha: what each pane costs, the memory its processes hold and how full
//! its agent's context is. Read off the input path every few seconds
//! (`read`); the window only draws the last answer.
//!
//! Memory adds up a pane's shell and everything under it, from one pass over
//! the process table. Context comes from the agent's own conversation file:
//! Claude Code writes each reply's token usage (not its window), Codex writes
//! its usage and its window. Only what was added to a file since the last
//! read is read again.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde_json::Value;

/// At most this much of a conversation file's end is read the first time.
const FIRST_READ: u64 = 4 * 1024 * 1024;

/// A session whose file wasn't found is looked for again after this long:
/// a miss walks the agent's whole session folder (~45 ms for Codex).
const RETRY_MISSING: std::time::Duration = std::time::Duration::from_secs(30);

/// A pane holding this much memory is called out; twice this, loudly.
pub(crate) const HEAVY_MEMORY: u64 = 1024 * 1024 * 1024;

/// A context this full is called out: 60% of a known window, else (Claude
/// Code doesn't write its window down) this many tokens. From 85% it is
/// about to run out.
pub(crate) const HEAVY_CONTEXT_PERCENT: u64 = 60;
pub(crate) const FULL_CONTEXT_PERCENT: u64 = 85;
const HEAVY_CONTEXT_TOKENS: u64 = 150_000;

/// How a pane is doing, with the reason when it isn't fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Health {
    Good,
    Watch(&'static str),
    Trouble(&'static str),
}

/// How full an agent's context is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Context {
    /// Tokens its latest request carried.
    pub used: u64,
    /// The model's window, when the agent writes it down.
    pub window: Option<u64>,
}

impl Context {
    /// Percent of the window in use, when the window is known.
    pub(crate) fn percent(self) -> Option<u64> {
        let window = self.window.filter(|window| *window > 0)?;
        Some((self.used.saturating_mul(100) / window).min(100))
    }

    /// "70%" when the window is known, else the tokens ("225k").
    pub(crate) fn short(self) -> String {
        match self.percent() {
            Some(percent) => format!("{percent}%"),
            None => tokens(self.used),
        }
    }

    /// Full enough to call out.
    pub(crate) fn heavy(self) -> bool {
        match self.percent() {
            Some(percent) => percent >= HEAVY_CONTEXT_PERCENT,
            None => self.used >= HEAVY_CONTEXT_TOKENS,
        }
    }
}

/// "950", "225k", "1.2M".
pub(crate) fn tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{}k", count / 1_000)
    } else {
        count.to_string()
    }
}

/// "45 MB", "1.2 GB", "16 GB".
pub(crate) fn bytes(count: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if count >= 10 * GIB {
        format!("{} GB", (count + GIB / 2) / GIB)
    } else if count >= GIB {
        format!("{:.1} GB", count as f64 / GIB as f64)
    } else {
        format!("{} MB", count.div_ceil(MIB))
    }
}

/// Rounded up to a whole MiB, like `bytes` shows it.
fn whole_mib(count: u64) -> u64 {
    const MIB: u64 = 1024 * 1024;
    count.div_ceil(MIB).saturating_mul(MIB)
}

/// "1.2G": `bytes` for a narrow row.
pub(crate) fn short_bytes(count: u64) -> String {
    bytes(count).replace(" GB", "G").replace(" MB", "M")
}

/// What one pane costs, and where its agent stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PaneUsage {
    /// Bytes its shell and everything under it hold.
    pub memory: Option<u64>,
    pub context: Option<Context>,
    /// Its heaviest process and that process's memory, once the pane is
    /// heavy (`HEAVY_MEMORY`) and that process isn't the agent itself.
    pub heaviest: Option<(String, u64)>,
    /// Seconds since its agent last wrote to its conversation, rounded down
    /// to the step it shows in (`age`), so a read only changes when that does.
    pub quiet_for: Option<u64>,
    /// The git branch of the folder its agent is in.
    pub branch: Option<Branch>,
}

/// A pane's git branch, read from the folder its agent is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Branch {
    pub name: String,
    /// The folder is a linked worktree, not the main checkout.
    pub worktree: bool,
}

/// Rounds seconds down to the step `age` shows: minutes, hours, then days.
fn shown_age(seconds: u64) -> u64 {
    let step = match seconds {
        0..3_600 => 60,
        3_600..86_400 => 3_600,
        _ => 86_400,
    };
    seconds / step * step
}

/// "12m", "2h", "3d"; None under a minute.
pub(crate) fn age(seconds: u64) -> Option<String> {
    match seconds {
        0..60 => None,
        60..3_600 => Some(format!("{}m", seconds / 60)),
        3_600..86_400 => Some(format!("{}h", seconds / 3_600)),
        _ => Some(format!("{}d", seconds / 86_400)),
    }
}

impl PaneUsage {
    /// The worst of its memory and its context.
    pub(crate) fn health(&self) -> Health {
        let memory = self.memory.unwrap_or(0);
        let percent = self.context.and_then(Context::percent);
        if memory >= HEAVY_MEMORY.saturating_mul(2) {
            return Health::Trouble("very heavy on RAM");
        }
        if percent.is_some_and(|percent| percent >= FULL_CONTEXT_PERCENT) {
            return Health::Trouble("context almost full");
        }
        if self.context.is_some_and(Context::heavy) {
            return Health::Watch(if percent.is_some() {
                "context filling up"
            } else {
                "big context"
            });
        }
        if memory >= HEAVY_MEMORY {
            return Health::Watch("heavy on RAM");
        }
        Health::Good
    }
}

/// One read: every pane, orcha itself and the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub panes: HashMap<String, PaneUsage>,
    /// orcha itself: its server and this window.
    pub orcha: Option<u64>,
    /// The machine's RAM.
    pub physical: Option<u64>,
}

impl Usage {
    /// orcha and every pane together; None where memory can't be read.
    pub(crate) fn total_memory(&self) -> Option<u64> {
        let panes: u64 = self.panes.values().filter_map(|pane| pane.memory).sum();
        self.orcha.map(|orcha| orcha.saturating_add(panes))
    }
}

/// Blocking: one `pane.list`, one pass over the process table, and the new
/// end of each agent's conversation file.
pub(crate) fn read() -> Option<Usage> {
    let panes = pane_list()?;
    let table = crate::platform::orcha_process_memory();
    let tree = ProcessTree::new(&table);
    let home = crate::platform::user_home_dir();
    let mut usage = Usage {
        physical: crate::platform::orcha_physical_memory(),
        ..Usage::default()
    };
    let mut server = None;
    let mut reader = READER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut seen = HashSet::new();
    // panes in the same folder share one branch read
    let mut branches: HashMap<&str, Option<Branch>> = HashMap::new();
    let now = std::time::SystemTime::now();
    for pane in &panes {
        let Some(id) = pane.get("pane_id").and_then(Value::as_str) else {
            continue;
        };
        let shell = pane
            .get("shell_pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok());
        // the shells' parent is orcha's server
        if server.is_none() {
            server = shell
                .and_then(|pid| tree.parent(pid))
                .filter(|pid| *pid > 1);
        }
        let file = home
            .as_deref()
            .and_then(|home| reader.context_of(pane, home, &mut seen));
        let quiet_for = file
            .and_then(|file| file.modified)
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|quiet| shown_age(quiet.as_secs()));
        let weight = shell.and_then(|pid| tree.weigh(pid));
        // the agent itself being the heaviest says nothing new
        let agent = pane.get("agent").and_then(Value::as_str);
        let heaviest = weight
            .filter(|weight| weight.total >= HEAVY_MEMORY)
            .and_then(|weight| {
                let name = readable_name(tree.name(weight.heaviest)?, weight.heaviest);
                let own = agent.is_some_and(|agent| name.eq_ignore_ascii_case(agent));
                (!own).then(|| (name, whole_mib(weight.heaviest_memory)))
            });
        // only agents get a branch line
        let branch = pane
            .get("agent")
            .is_some()
            .then(|| {
                pane.get("foreground_cwd")
                    .or_else(|| pane.get("cwd"))
                    .and_then(Value::as_str)
            })
            .flatten()
            .and_then(|cwd| {
                branches
                    .entry(cwd)
                    .or_insert_with(|| branch_of(Path::new(cwd)))
                    .clone()
            });
        usage.panes.insert(
            id.to_owned(),
            PaneUsage {
                memory: weight.map(|weight| whole_mib(weight.total)),
                context: file.and_then(|file| file.context),
                heaviest,
                quiet_for,
                branch,
            },
        );
    }
    reader.keep_only(&seen);
    usage.orcha = tree.memory(std::process::id()).map(|window| {
        whole_mib(window.saturating_add(server.and_then(|pid| tree.memory(pid)).unwrap_or(0)))
    });
    Some(usage)
}

/// A process name people recognise. Some tools run a file named after
/// their version (Claude Code: `~/.local/share/claude/versions/2.1.317`):
/// those are called by the folder they live in.
fn readable_name(name: &str, pid: u32) -> String {
    if !looks_like_version(name) {
        return name.to_owned();
    }
    crate::platform::orcha_process_exe(pid)
        .and_then(|exe| named_folder(&exe))
        .unwrap_or_else(|| name.to_owned())
}

/// "2.1.317", "1.0": digits and dots, at least one dot.
fn looks_like_version(name: &str) -> bool {
    name.contains('.') && name.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
}

/// The nearest folder above `exe` that names the tool, past version
/// folders and the usual `versions`/`bin`.
fn named_folder(exe: &Path) -> Option<String> {
    exe.ancestors()
        .skip(1)
        .filter_map(|folder| folder.file_name()?.to_str())
        .find(|name| !looks_like_version(name) && !matches!(*name, "versions" | "bin"))
        .map(str::to_owned)
}

/// The branch of the repository `folder` is in, from git's own files (no
/// `git` process); a detached HEAD has none.
fn branch_of(folder: &Path) -> Option<Branch> {
    let name = crate::workspace::git_branch(folder)?;
    let worktree =
        crate::workspace::git_space_metadata(folder).is_some_and(|space| space.is_linked_worktree);
    Some(Branch { name, worktree })
}

fn pane_list() -> Option<Vec<Value>> {
    let request = crate::api::schema::Request {
        id: "orcha:usage".into(),
        method: crate::api::schema::Method::PaneList(crate::api::schema::PaneListParams::default()),
    };
    let client = crate::api::client::ApiClient::for_target(
        crate::api::client::ConnectionTarget::SocketPath(crate::session::active_api_socket_path()),
    );
    let mut reply = client
        .request_value_with_timeout(&request, std::time::Duration::from_secs(2))
        .ok()?;
    match reply.get_mut("result")?.get_mut("panes")?.take() {
        Value::Array(panes) => Some(panes),
        _ => None,
    }
}

/// The process table as a tree.
struct ProcessTree<'a> {
    memory: HashMap<u32, u64>,
    parent: HashMap<u32, u32>,
    names: HashMap<u32, &'a str>,
    children: HashMap<u32, Vec<u32>>,
}

/// What runs under a pane: all of it, and its heaviest process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Weight {
    total: u64,
    heaviest: u32,
    heaviest_memory: u64,
}

impl<'a> ProcessTree<'a> {
    fn new(table: &'a [crate::platform::ProcessMemory]) -> Self {
        let mut tree = Self {
            memory: HashMap::with_capacity(table.len()),
            parent: HashMap::with_capacity(table.len()),
            names: HashMap::with_capacity(table.len()),
            children: HashMap::new(),
        };
        for process in table {
            tree.memory.insert(process.pid, process.memory);
            tree.parent.insert(process.pid, process.ppid);
            tree.names.insert(process.pid, process.name.as_str());
            tree.children
                .entry(process.ppid)
                .or_default()
                .push(process.pid);
        }
        tree
    }

    fn memory(&self, pid: u32) -> Option<u64> {
        self.memory.get(&pid).copied()
    }

    fn name(&self, pid: u32) -> Option<&'a str> {
        self.names
            .get(&pid)
            .copied()
            .filter(|name| !name.is_empty())
    }

    fn parent(&self, pid: u32) -> Option<u32> {
        self.parent.get(&pid).copied()
    }

    /// `root` and everything under it; None when `root` isn't running.
    fn weigh(&self, root: u32) -> Option<Weight> {
        self.memory(root)?;
        let mut weight = Weight {
            total: 0,
            heaviest: root,
            heaviest_memory: 0,
        };
        let mut seen = HashSet::new();
        let mut next = vec![root];
        while let Some(pid) = next.pop() {
            if !seen.insert(pid) {
                continue;
            }
            let memory = self.memory(pid).unwrap_or(0);
            weight.total = weight.total.saturating_add(memory);
            if memory > weight.heaviest_memory {
                weight.heaviest = pid;
                weight.heaviest_memory = memory;
            }
            if let Some(children) = self.children.get(&pid) {
                next.extend(children.iter().copied().filter(|child| *child != pid));
            }
        }
        Some(weight)
    }
}

static READER: LazyLock<Mutex<Reader>> = LazyLock::new(|| Mutex::new(Reader::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Claude,
    Codex,
}

/// Where each session's conversation file is, and how far each file was
/// read, kept between reads.
#[derive(Default)]
struct Reader {
    files: HashMap<String, PathBuf>,
    reads: HashMap<PathBuf, FileRead>,
    /// Sessions whose file wasn't found, and when it was last looked for.
    missing: HashMap<String, std::time::Instant>,
}

#[derive(Debug, Clone, Copy)]
struct FileRead {
    /// Read up to here; always the start of a line once `aligned`.
    offset: u64,
    aligned: bool,
    context: Option<Context>,
}

/// What a conversation file says: the context, and when it last changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileState {
    context: Option<Context>,
    modified: Option<std::time::SystemTime>,
}

/// What a stretch of a conversation file says about the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    Nothing,
    Found(Context),
    /// The conversation was compacted after its last usage.
    Reset,
}

impl Reader {
    fn context_of(
        &mut self,
        pane: &Value,
        home: &Path,
        seen: &mut HashSet<String>,
    ) -> Option<FileState> {
        // the agent that owns the session decides how its file reads
        let session_info = pane.get("agent_session")?;
        let agent = session_info
            .get("agent")
            .or_else(|| pane.get("agent"))
            .and_then(Value::as_str)?;
        let kind = match agent {
            "claude" => Kind::Claude,
            "codex" => Kind::Codex,
            _ => return None,
        };
        let session = session_info.get("value")?.as_str()?;
        // the id becomes part of a file name: nothing that could leave the folder
        if !crate::orcha_handoff::plain_session_id(session) {
            return None;
        }
        let key = format!("{agent}:{session}");
        seen.insert(key.clone());
        let path = match self.files.get(&key) {
            Some(path) if path.is_file() => path.clone(),
            _ => {
                let looked = self.missing.get(&key);
                if looked.is_some_and(|at| at.elapsed() < RETRY_MISSING) {
                    return None;
                }
                let found = match kind {
                    Kind::Claude => crate::orcha_handoff::claude_file(session, home),
                    Kind::Codex => crate::orcha_handoff::codex_file(session, home),
                };
                let Some(found) = found else {
                    self.missing.insert(key, std::time::Instant::now());
                    return None;
                };
                self.missing.remove(&key);
                self.files.insert(key, found.clone());
                found
            }
        };
        self.update(&path, kind)
    }

    /// Reads what was added to `path` since last time.
    fn update(&mut self, path: &Path, kind: Kind) -> Option<FileState> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok();
        let context = self.read_added(path, kind, meta.len())?;
        Some(FileState { context, modified })
    }

    /// The context, after reading what was added to `path` up to `len`.
    fn read_added(&mut self, path: &Path, kind: Kind, len: u64) -> Option<Option<Context>> {
        let fresh = FileRead {
            offset: len.saturating_sub(FIRST_READ),
            aligned: len <= FIRST_READ,
            context: None,
        };
        let read = self.reads.entry(path.to_owned()).or_insert(fresh);
        // rewritten from the start: read it again
        if len < read.offset {
            *read = fresh;
        }
        if len == read.offset {
            return Some(read.context);
        }
        let mut file = std::fs::File::open(path).ok()?;
        file.seek(SeekFrom::Start(read.offset)).ok()?;
        let mut added = Vec::new();
        file.take(len - read.offset).read_to_end(&mut added).ok()?;
        // whole lines only: a line still being written waits for next time
        let Some(end) = added.iter().rposition(|byte| *byte == b'\n') else {
            return Some(read.context);
        };
        let mut start = 0;
        if !read.aligned {
            // the first read starts inside a line: skip to the next one
            start = added[..=end]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(end + 1, |newline| newline + 1);
        }
        let text = String::from_utf8_lossy(&added[start..=end]);
        match scan(kind, &text) {
            Scan::Found(context) => read.context = Some(context),
            Scan::Reset => read.context = None,
            Scan::Nothing => {}
        }
        read.offset += end as u64 + 1;
        read.aligned = true;
        Some(read.context)
    }

    /// Forgets sessions no pane runs any more.
    fn keep_only(&mut self, seen: &HashSet<String>) {
        self.files.retain(|key, _| seen.contains(key));
        self.missing.retain(|key, _| seen.contains(key));
        let files: HashSet<&PathBuf> = self.files.values().collect();
        self.reads.retain(|path, _| files.contains(path));
    }
}

fn scan(kind: Kind, text: &str) -> Scan {
    match kind {
        Kind::Claude => scan_claude(text),
        Kind::Codex => scan_codex(text),
    }
}

/// Claude Code: the latest main-chat reply's usage (input, cache and output
/// tokens); a compaction after it starts over.
fn scan_claude(text: &str) -> Scan {
    // from the end: the latest reply or compaction decides
    for line in text.lines().rev() {
        let side = line.contains("\"isSidechain\":true");
        if line.contains("\"compact_boundary\"") && !side {
            return Scan::Reset;
        }
        if side || !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = entry
            .get("message")
            .and_then(|message| message.get("usage"))
        else {
            continue;
        };
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let used = count("input_tokens")
            .saturating_add(count("cache_creation_input_tokens"))
            .saturating_add(count("cache_read_input_tokens"))
            .saturating_add(count("output_tokens"));
        // replies orcha didn't send (errors, local notes) carry no tokens
        if used > 0 {
            return Scan::Found(Context { used, window: None });
        }
    }
    Scan::Nothing
}

/// Codex: the latest `token_count` event's last request and the window.
fn scan_codex(text: &str) -> Scan {
    // from the end: the latest count decides
    for line in text.lines().rev() {
        if !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(info) = entry
            .get("payload")
            .and_then(|payload| payload.get("info"))
            .filter(|info| info.is_object())
        else {
            continue;
        };
        let Some(used) = info
            .get("last_token_usage")
            .and_then(|last| last.get("total_tokens"))
            .and_then(Value::as_u64)
        else {
            continue;
        };
        return Scan::Found(Context {
            used,
            window: info.get("model_context_window").and_then(Value::as_u64),
        });
    }
    Scan::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ProcessMemory;

    fn process(pid: u32, ppid: u32, memory: u64) -> ProcessMemory {
        ProcessMemory {
            pid,
            ppid,
            name: format!("p{pid}"),
            memory,
        }
    }

    #[test]
    fn a_pane_counts_its_shell_and_everything_under_it() {
        let table = [
            process(1, 0, 5),
            process(10, 1, 100),
            process(20, 10, 30),
            process(21, 10, 40),
            process(30, 20, 7),
            process(99, 1, 1_000),
        ];
        let tree = ProcessTree::new(&table);
        let weight = tree.weigh(10).expect("running");
        assert_eq!((weight.total, weight.heaviest), (177, 10));
        assert_eq!(tree.weigh(20).map(|weight| weight.heaviest), Some(20));
        assert_eq!(tree.weigh(30).map(|weight| weight.total), Some(7));
        assert_eq!(tree.weigh(4242), None);
        assert_eq!(tree.name(21), Some("p21"));
        assert_eq!(tree.parent(10), Some(1));
    }

    #[test]
    fn claude_context_is_the_latest_main_reply_and_compaction_resets_it() {
        let reply = |input: u64, cache: u64, side: bool| {
            format!(
                r#"{{"type":"assistant","isSidechain":{side},"message":{{"usage":{{"input_tokens":{input},"cache_creation_input_tokens":0,"cache_read_input_tokens":{cache},"output_tokens":10}}}}}}"#
            )
        };
        let text = [
            reply(5, 1_000, false),
            reply(1, 50_000, true),
            r#"{"type":"user","message":{"content":"usage of the assistant"}}"#.to_owned(),
            reply(2, 2_000, false),
            r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}"#.to_owned(),
        ]
        .join("\n");
        assert_eq!(
            scan_claude(&text),
            Scan::Found(Context {
                used: 2_012,
                window: None
            })
        );
        let compacted = format!(
            "{text}\n{}",
            r#"{"type":"system","subtype":"compact_boundary","isSidechain":false}"#
        );
        assert_eq!(scan_claude(&compacted), Scan::Reset);
        assert_eq!(scan_claude("not json\n"), Scan::Nothing);
    }

    #[test]
    fn codex_context_comes_with_its_window() {
        let text = [
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":332179},"model_context_window":475000}}}"#,
        ]
        .join("\n");
        let Scan::Found(context) = scan_codex(&text) else {
            panic!("no context in {text}");
        };
        assert_eq!(context.percent(), Some(69));
        assert_eq!(context.short(), "69%");
    }

    #[test]
    fn only_whole_new_lines_are_read_again() {
        let dir = std::env::temp_dir().join(format!("orcha-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let path = dir.join("rollout.jsonl");
        let event = |used: u64| {
            format!(
                r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":{used}}},"model_context_window":1000}}}}}}"#
            )
        };
        std::fs::write(&path, format!("{}\n", event(100))).expect("write");
        let mut reader = Reader::default();
        assert_eq!(
            reader
                .update(&path, Kind::Codex)
                .and_then(|state| state.context)
                .map(|c| c.used),
            Some(100)
        );
        // half a line: not yet
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open");
        let next = event(300);
        std::io::Write::write_all(&mut file, &next.as_bytes()[..20]).expect("append");
        assert_eq!(
            reader
                .update(&path, Kind::Codex)
                .and_then(|state| state.context)
                .map(|c| c.used),
            Some(100)
        );
        std::io::Write::write_all(&mut file, format!("{}\n", &next[20..]).as_bytes())
            .expect("append");
        assert_eq!(
            reader
                .update(&path, Kind::Codex)
                .and_then(|state| state.context)
                .map(|c| c.used),
            Some(300)
        );
        // nothing new: the last answer holds
        assert_eq!(
            reader
                .update(&path, Kind::Codex)
                .and_then(|state| state.context)
                .map(|c| c.used),
            Some(300)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_session_file_is_looked_for_again_only_after_a_while() {
        let home = std::env::temp_dir().join(format!("orcha-usage-miss-{}", std::process::id()));
        let project = home.join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&project).expect("temp folder");
        let pane = serde_json::json!({
            "pane_id": "p1",
            "agent_session": {"agent": "claude", "value": "abc-123"},
        });
        let mut reader = Reader::default();
        let mut seen = HashSet::new();
        assert_eq!(reader.context_of(&pane, &home, &mut seen), None);
        let reply =
            r#"{"type":"assistant","message":{"usage":{"input_tokens":7,"output_tokens":3}}}"#;
        std::fs::write(project.join("abc-123.jsonl"), format!("{reply}\n")).expect("write");
        // still within the wait: not looked for again
        assert_eq!(reader.context_of(&pane, &home, &mut seen), None);
        let long_ago = std::time::Instant::now().checked_sub(RETRY_MISSING * 2);
        if let Some(long_ago) = long_ago {
            reader.missing.insert("claude:abc-123".into(), long_ago);
            assert_eq!(
                reader
                    .context_of(&pane, &home, &mut seen)
                    .and_then(|state| state.context)
                    .map(|c| c.used),
                Some(10)
            );
            assert!(reader.missing.is_empty());
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn versioned_executables_go_by_their_folder() {
        assert!(looks_like_version("2.1.317"));
        assert!(!looks_like_version("node") && !looks_like_version("python3.13"));
        assert_eq!(
            named_folder(Path::new("/Users/me/.local/share/claude/versions/2.1.317")),
            Some("claude".to_owned())
        );
        assert_eq!(
            named_folder(Path::new("/opt/tool/1.2/bin/1.2.3")),
            Some("tool".to_owned())
        );
        assert_eq!(readable_name("xcodebuild", 1), "xcodebuild");
    }

    #[test]
    fn ages_read_in_their_biggest_whole_unit() {
        assert_eq!(age(59), None);
        assert_eq!(age(shown_age(12 * 60 + 59)), Some("12m".to_owned()));
        assert_eq!(shown_age(12 * 60 + 59), 12 * 60);
        assert_eq!(age(shown_age(2 * 3_600 + 1_799)), Some("2h".to_owned()));
        assert_eq!(age(shown_age(3 * 86_400 + 5)), Some("3d".to_owned()));
    }

    #[test]
    fn memory_is_kept_in_whole_mib() {
        const MIB: u64 = 1024 * 1024;
        assert_eq!(whole_mib(0), 0);
        assert_eq!(whole_mib(1), MIB);
        assert_eq!(whole_mib(MIB), MIB);
        assert_eq!(whole_mib(MIB + 4096), 2 * MIB);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(bytes(45 * 1024 * 1024), "45 MB");
        assert_eq!(bytes(1), "1 MB");
        assert_eq!(bytes(HEAVY_MEMORY + HEAVY_MEMORY / 5), "1.2 GB");
        assert_eq!(short_bytes(HEAVY_MEMORY + HEAVY_MEMORY / 5), "1.2G");
        assert_eq!(bytes(16 * HEAVY_MEMORY), "16 GB");
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(225_123), "225k");
        assert_eq!(tokens(1_250_000), "1.2M");
        let unknown = Context {
            used: 225_000,
            window: None,
        };
        assert_eq!(
            (unknown.percent(), unknown.short()),
            (None, "225k".to_owned())
        );
        assert!(unknown.heavy());
        let roomy = Context {
            used: 100,
            window: Some(1_000),
        };
        assert!(!roomy.heavy());
        assert!(Context {
            used: 600,
            window: Some(1_000)
        }
        .heavy());
    }

    #[test]
    fn health_names_the_worst_problem() {
        let pane = |memory: u64, used: u64, window: Option<u64>| PaneUsage {
            memory: Some(memory),
            context: Some(Context { used, window }),
            ..PaneUsage::default()
        };
        assert_eq!(pane(10, 100, Some(1_000)).health(), Health::Good);
        assert_eq!(
            pane(HEAVY_MEMORY, 100, Some(1_000)).health(),
            Health::Watch("heavy on RAM")
        );
        assert_eq!(
            pane(10, 700, Some(1_000)).health(),
            Health::Watch("context filling up")
        );
        assert_eq!(
            pane(10, 200_000, None).health(),
            Health::Watch("big context")
        );
        assert_eq!(
            pane(HEAVY_MEMORY, 900, Some(1_000)).health(),
            Health::Trouble("context almost full")
        );
        assert_eq!(
            pane(HEAVY_MEMORY * 2, 100, Some(1_000)).health(),
            Health::Trouble("very heavy on RAM")
        );
        assert_eq!(PaneUsage::default().health(), Health::Good);
    }

    #[test]
    fn total_memory_needs_orcha_itself() {
        let mut usage = Usage::default();
        usage.panes.insert(
            "p1".into(),
            PaneUsage {
                memory: Some(10),
                ..PaneUsage::default()
            },
        );
        assert_eq!(usage.total_memory(), None);
        usage.orcha = Some(5);
        assert_eq!(usage.total_memory(), Some(15));
    }
}
