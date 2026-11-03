//! orcha: hand a pane's conversation to another agent (HAND OFF and
//! CONTINUE ON, docs/orcha/SPEC.md §3.4).
//!
//! The old agent is not asked for anything, so this works when it is out of
//! fuel. Notes are built from its conversation file (Claude Code's
//! `<config>/projects/*/<session>.jsonl`, Codex's `~/.codex/sessions/**`),
//! else from the pane's recent screen (Cursor keeps its chats in SQLite),
//! plus the folder's git state. They are written to
//! `~/.cache/orcha/handoff/` (owner-only, kept two weeks) and the new agent
//! opens next to the old one, told to read them, sum up where the work
//! stands and wait for the user before going on.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Keep the notes readable in one go.
const FULL_CHARS: usize = 120_000;
const SUMMARY_CHARS: usize = 60_000;
/// Screen lines read when there is no conversation file.
const SCREEN_LINES: &str = "800";
/// Files the old agent changed, at most.
const MAX_FILES: usize = 40;

/// The agents a conversation can move to, from the accounts' registry.
const AGENTS: &[&crate::orcha_accounts::AgentKind] = &crate::orcha_accounts::HANDOFF_AGENTS;

/// A conversation can move to an agent of this kind.
pub(crate) fn can_hand_off_to(kind: &str) -> bool {
    agent(kind).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The notes carry the whole conversation.
    Full,
    /// The notes carry only its latest part, so they read faster.
    Summary,
}

impl Mode {
    pub(crate) fn from_notes(notes: &str) -> Option<Self> {
        match notes {
            "full" => Some(Self::Full),
            "summary" => Some(Self::Summary),
            _ => None,
        }
    }

    pub(crate) fn parse(mode: &str) -> Self {
        if mode == "summary" {
            Self::Summary
        } else {
            Self::Full
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

/// What a conversation file tells: its text turns and the files changed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Transcript {
    turns: Vec<(Role, String)>,
    /// Most recently changed first, no repeats.
    files: Vec<String>,
}

impl Transcript {
    fn touch(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        self.files.retain(|known| known != path);
        self.files.insert(0, path.to_owned());
    }
}

/// The agent's display name and command for a kind.
fn agent(kind: &str) -> Option<(&'static str, &'static str)> {
    AGENTS
        .iter()
        .find(|agent| agent.kind == kind)
        .map(|agent| (agent.command, agent.name))
}

/// The display name of an agent kind ("Codex"), else the kind itself.
fn agent_name(kind: &str) -> String {
    agent(kind).map_or_else(
        || {
            let mut chars = kind.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        },
        |(_, name)| name.to_owned(),
    )
}

/// The plain text of a message's content: a string or a list of blocks.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("text" | "input_text" | "output_text")
                )
            })
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Text a person or agent wrote, not command output or injected context.
fn keep_turn(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('<') && !text.starts_with("Caveat:")
}

/// Claude Code's `<session>.jsonl`.
fn parse_claude(text: &str) -> Transcript {
    let mut out = Transcript::default();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry.get("type").and_then(Value::as_str);
        let role = match kind {
            Some("user") => Role::User,
            Some("assistant") => Role::Agent,
            _ => continue,
        };
        // meta lines and subagent chats are not the conversation
        if entry.get("isMeta").and_then(Value::as_bool) == Some(true)
            || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let content = entry
            .get("message")
            .and_then(|message| message.get("content"));
        if let Some(Value::Array(blocks)) = content {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                    let input = block.get("input");
                    let path = input
                        .and_then(|input| input.get("file_path"))
                        .or_else(|| input.and_then(|input| input.get("notebook_path")))
                        .and_then(Value::as_str);
                    if let Some(path) = path {
                        out.touch(path);
                    }
                }
            }
        }
        let text = text_of(content);
        let text = text.trim();
        if keep_turn(text) {
            out.turns.push((role, text.to_owned()));
        }
    }
    out
}

/// Files named by an `apply_patch` body.
fn patched_files(patch: &str, out: &mut Transcript) {
    for line in patch.lines() {
        for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(path) = line.trim().strip_prefix(marker) {
                out.touch(path);
            }
        }
    }
}

/// Codex's `rollout-*-<session>.jsonl`.
fn parse_codex(text: &str) -> Transcript {
    let mut out = Transcript::default();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = entry.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(Value::as_str) {
            Some("message") => {
                let role = match payload.get("role").and_then(Value::as_str) {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Agent,
                    _ => continue,
                };
                let text = text_of(payload.get("content"));
                let text = text.trim();
                if keep_turn(text) {
                    out.turns.push((role, text.to_owned()));
                }
            }
            Some("function_call" | "custom_tool_call") => {
                for key in ["arguments", "input"] {
                    if let Some(body) = payload.get(key).and_then(Value::as_str) {
                        patched_files(body, &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn home() -> Option<PathBuf> {
    crate::platform::user_home_dir()
}

fn newest(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().max_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    })
}

pub(crate) fn claude_file(session: &str, home: &Path) -> Option<PathBuf> {
    let name = format!("{session}.jsonl");
    let mut found = Vec::new();
    for folder in crate::orcha_accounts::claude_config_folders(home) {
        let Ok(projects) = std::fs::read_dir(folder.join("projects")) else {
            continue;
        };
        for project in projects.flatten() {
            let path = project.path().join(&name);
            if path.is_file() {
                found.push(path);
            }
        }
    }
    newest(found)
}

/// `~/.codex/sessions/YYYY/MM/DD/rollout-…-<session>.jsonl`.
pub(crate) fn codex_file(session: &str, home: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize, session: &str, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, session, found);
                }
            } else if path.extension().is_some_and(|ext| ext == "jsonl")
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(session))
            {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(
        &home.join(".codex").join("sessions"),
        4,
        session,
        &mut found,
    );
    newest(found)
}

/// A session id safe to put in a file name or a shell line as it is:
/// letters, digits, `-` and `_` only (Claude and Codex ids are UUIDs).
pub(crate) fn plain_session_id(session: &str) -> bool {
    !session.is_empty()
        && session
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn transcript(source: &str, session: &str, home: &Path) -> Option<Transcript> {
    // the id becomes part of a file name: nothing that could leave the folder
    if !plain_session_id(session) {
        return None;
    }
    let (path, parse): (PathBuf, fn(&str) -> Transcript) = match source {
        "claude" => (claude_file(session, home)?, parse_claude),
        "codex" => (codex_file(session, home)?, parse_codex),
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    let parsed = parse(&String::from_utf8_lossy(&bytes));
    (!parsed.turns.is_empty()).then_some(parsed)
}

/// This app's CLI against this session; the `result` of its JSON reply.
pub(crate) fn herdr_json(args: &[&str]) -> Option<Value> {
    let output = crate::orcha_cli::OrchaCli::new(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut value: Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(value.get_mut("result")?.take())
}

/// This app's CLI against this session, without a shell between.
pub(crate) fn herdr(args: &[&str]) -> Command {
    crate::orcha_cli::OrchaCli::new(args).into_command()
}

fn git(cwd: &str, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    crate::platform::configure_background_command(&mut command);
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// The repository's state, as the notes show it.
#[derive(Debug, Default)]
struct RepoState {
    branch: String,
    status: String,
    diff_stat: String,
    commits: String,
}

impl RepoState {
    fn read(cwd: &str) -> Self {
        Self {
            branch: git(cwd, &["branch", "--show-current"]),
            status: git(cwd, &["status", "--short"]),
            diff_stat: git(cwd, &["diff", "HEAD", "--stat"]),
            commits: git(cwd, &["log", "--oneline", "-5"]),
        }
    }
}

/// Everything the notes are made of, read beforehand so building them is
/// pure.
#[derive(Debug, Default)]
struct Inputs {
    source: String,
    cwd: String,
    task: String,
    mode: Option<Mode>,
    transcript: Option<Transcript>,
    screen: String,
    repo: RepoState,
    /// The checklist the old agent wrote before running low, when there is one.
    checklist: Option<String>,
}

/// The last `limit` bytes of `text`, cut at a char boundary.
fn tail(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The first `limit` bytes of `text`, cut at a char boundary.
fn head(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn fenced(lines: &mut Vec<String>, text: &str) {
    lines.push("```".to_owned());
    lines.push(text.to_owned());
    lines.push("```".to_owned());
    lines.push(String::new());
}

fn build_notes(inputs: &Inputs) -> String {
    let source = agent_name(&inputs.source);
    let mode = inputs.mode.unwrap_or(Mode::Full);
    let title = if inputs.task.is_empty() {
        "work in progress"
    } else {
        &inputs.task
    };
    let mut lines = vec![
        format!("# Hand-off: {title}"),
        String::new(),
        format!(
            "You are taking over work another coding agent ({source}) started in `{}`.",
            inputs.cwd
        ),
        "It stopped (usually out of plan limits), not because the work is done.".to_owned(),
        String::new(),
        "Read these notes and check the repository state yourself. Then sum up where the \
         work stands in five bullets: goal, what is done, what is left, open questions, the \
         next step you suggest."
            .to_owned(),
        String::new(),
        "**Do not continue the work yet.** Change no files and run nothing that changes \
         state until the user tells you to go on."
            .to_owned(),
        String::new(),
    ];
    if let Some(checklist) = &inputs.checklist {
        lines.push(format!(
            "The previous agent left a checklist in `{checklist}`: read it too."
        ));
        lines.push(String::new());
    }

    let repo = &inputs.repo;
    if !repo.branch.is_empty() || !repo.status.is_empty() || !repo.commits.is_empty() {
        lines.push("## Repository".to_owned());
        lines.push(String::new());
        let branch = if repo.branch.is_empty() {
            "(detached)"
        } else {
            &repo.branch
        };
        lines.push(format!("- branch: `{branch}`"));
        lines.push(String::new());
        if !repo.status.is_empty() {
            lines.push("Uncommitted changes:".to_owned());
            lines.push(String::new());
            fenced(&mut lines, head(&repo.status, 4000));
        }
        if !repo.diff_stat.is_empty() {
            fenced(&mut lines, tail(&repo.diff_stat, 4000));
        }
        if !repo.commits.is_empty() {
            lines.push("Latest commits:".to_owned());
            lines.push(String::new());
            fenced(&mut lines, &repo.commits);
        }
    }

    match &inputs.transcript {
        Some(transcript) => {
            if !transcript.files.is_empty() {
                lines.push(format!("## Files {source} changed (latest first)"));
                lines.push(String::new());
                for file in transcript.files.iter().take(MAX_FILES) {
                    lines.push(format!("- `{file}`"));
                }
                lines.push(String::new());
            }
            let first = transcript
                .turns
                .iter()
                .find(|(role, _)| *role == Role::User)
                .map(|(_, text)| text.as_str())
                .unwrap_or("");
            let last = transcript
                .turns
                .iter()
                .rev()
                .find(|(role, _)| *role == Role::User)
                .map(|(_, text)| text.as_str())
                .unwrap_or("");
            lines.push("## The goal (first request)".to_owned());
            lines.push(String::new());
            lines.push(head(first, 6000).to_owned());
            lines.push(String::new());
            if !last.is_empty() && last != first {
                lines.push("## The latest request".to_owned());
                lines.push(String::new());
                lines.push(head(last, 6000).to_owned());
                lines.push(String::new());
            }
            lines.push("## Conversation (oldest first, text only)".to_owned());
            lines.push(String::new());
            let body: Vec<String> = transcript
                .turns
                .iter()
                .map(|(role, text)| {
                    let who = match role {
                        Role::User => "User",
                        Role::Agent => source.as_str(),
                    };
                    format!("**{who}:** {text}\n")
                })
                .collect();
            let joined = body.join("\n");
            let limit = match mode {
                Mode::Full => FULL_CHARS,
                Mode::Summary => SUMMARY_CHARS,
            };
            if joined.len() > limit {
                lines.push("…(earlier turns cut)…".to_owned());
                lines.push(String::new());
                lines.push(tail(&joined, limit).to_owned());
            } else {
                lines.push(joined);
            }
        }
        None if !inputs.screen.trim().is_empty() => {
            lines.push(format!("## {source}'s screen (its latest output)"));
            lines.push(String::new());
            lines.push(
                "The conversation file could not be read, so this is the end of what the \
                 pane showed."
                    .to_owned(),
            );
            lines.push(String::new());
            let limit = match mode {
                Mode::Full => FULL_CHARS,
                Mode::Summary => SUMMARY_CHARS,
            };
            fenced(&mut lines, tail(inputs.screen.trim(), limit));
        }
        None => {
            lines.push("## Conversation".to_owned());
            lines.push(String::new());
            lines.push(
                "(the old agent's conversation could not be read; sum up from the repository \
                 state and the task above)"
                    .to_owned(),
            );
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

/// Notes older than this are deleted when new ones are written: they hold
/// whole conversations.
const NOTES_KEPT: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);

/// Writes `notes` to a new owner-only file in `folder` (made owner-only
/// too) and deletes notes older than two weeks. Returns the new file.
fn save_notes(
    folder: &Path,
    target: &str,
    notes: &str,
    now: time::OffsetDateTime,
) -> std::io::Result<PathBuf> {
    use std::io::Write;
    if let Some(parent) = folder.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match crate::platform::create_remote_private_dir(folder) {
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => return Err(err),
        _ => {}
    }
    crate::platform::restrict_private_state_path(folder)?;
    prune_notes(folder, std::time::SystemTime::from(now))?;
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    // two hand-offs in the same second each get their own file
    for attempt in 0..100 {
        let name = match attempt {
            0 => format!("{stamp}-{target}.md"),
            n => format!("{stamp}-{target}-{n}.md"),
        };
        let path = folder.join(name);
        let mut file = match crate::platform::create_private_state_file(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        if let Err(err) = crate::platform::restrict_private_state_path(&path)
            .and_then(|()| file.write_all(notes.as_bytes()))
        {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(err);
        }
        return Ok(path);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many hand-offs at once",
    ))
}

/// Deletes the hand-off notes in `folder` last changed before `now` minus
/// two weeks. Only files named like notes are touched.
fn prune_notes(folder: &Path, now: std::time::SystemTime) -> std::io::Result<()> {
    let entries = std::fs::read_dir(folder)?;
    let cutoff = now.checked_sub(NOTES_KEPT);
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !note_file_name(&name) || !entry.file_type()?.is_file() {
            continue;
        }
        // Notes from older builds were world-readable. Tighten those too,
        // including on Windows where directory traversal can bypass a DACL.
        crate::platform::restrict_private_state_path(&entry.path())?;
        let old = entry
            .metadata()?
            .modified()
            .ok()
            .zip(cutoff)
            .is_some_and(|(modified, cutoff)| modified < cutoff);
        if old {
            if let Err(err) = std::fs::remove_file(entry.path()) {
                tracing::warn!(%err, file = %entry.path().display(), "could not delete old hand-off notes");
            }
        }
    }
    Ok(())
}

/// Names produced by save_notes, including the same-second counter.
fn note_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".md") else {
        return false;
    };
    let mut parts = stem.split('-');
    let digits = |part: Option<&str>, count: usize| {
        part.is_some_and(|part| part.len() == count && part.bytes().all(|b| b.is_ascii_digit()))
    };
    if !digits(parts.next(), 8)
        || !digits(parts.next(), 6)
        || !parts.next().is_some_and(can_hand_off_to)
    {
        return false;
    }
    if parts
        .next()
        .is_some_and(|counter| counter.is_empty() || !counter.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    parts.next().is_none()
}

/// A command found on PATH or in the folders agent installers use before
/// PATH knows them.
pub(crate) fn command_installed(command: &str) -> bool {
    let names = crate::platform::orcha_command_file_names(command);
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = home() {
        folders.extend(crate::platform::orcha_agent_install_dirs(&home));
    }
    folders
        .iter()
        .any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

/// The kinds a conversation of `source` can move to: the other installed
/// agents.
pub(crate) fn targets(source: Option<&str>) -> Vec<&'static str> {
    AGENTS
        .iter()
        .filter(|agent| Some(agent.kind) != source)
        .filter(|agent| command_installed(agent.command))
        .map(|agent| agent.kind)
        .collect()
}

/// Types `text` into `pane` and presses enter (asks its agent something).
pub(crate) fn send_prompt(pane: &str, text: &str) -> bool {
    use crate::orcha_cli::{succeeded, OrchaCli};
    succeeded(OrchaCli::new(&["pane", "send-text", pane, text]).quiet())
        && succeeded(OrchaCli::new(&["pane", "send-keys", pane, "enter"]).quiet())
}

/// Builds the notes for `pane`, opens `target` next to it and tells it to
/// read them and sum them up, then wait. Blocking (runs off the input
/// thread). `claude_dir` starts a Claude target in that account's sign-in
/// folder. Ok is (message, the new pane's id); Err says what went wrong.
pub(crate) fn hand_off_to(
    pane: &str,
    target: &str,
    mode: Mode,
    claude_dir: Option<&str>,
) -> Result<(String, String), String> {
    let (command, target_name) = agent(target).ok_or_else(|| format!("unknown agent {target}"))?;
    let home = home().ok_or("no home folder")?;
    let info = herdr_json(&["pane", "get", pane])
        .and_then(|mut result| Some(result.get_mut("pane")?.take()))
        .ok_or("the pane is gone")?;
    let field = |key: &str| {
        info.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let source = info
        .get("agent")
        .and_then(Value::as_str)
        .unwrap_or("agent")
        .to_owned();
    let session = info
        .get("agent_session")
        .and_then(|session| session.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let cwd = [field("foreground_cwd"), field("cwd")]
        .into_iter()
        .find(|cwd| !cwd.is_empty())
        .unwrap_or_else(|| home.to_string_lossy().into_owned());
    let task = info
        .get("tokens")
        .and_then(|tokens| tokens.get("task"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| field("terminal_title_stripped"));
    let transcript = (!session.is_empty())
        .then(|| transcript(&source, &session, &home))
        .flatten();
    let screen = if transcript.is_none() {
        crate::orcha_cli::OrchaCli::new(&[
            "pane",
            "read",
            pane,
            "--source",
            "recent-unwrapped",
            "--lines",
            SCREEN_LINES,
            "--format",
            "text",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
    } else {
        String::new()
    };
    let checklist_file = crate::config::Config::load().config.orcha.checklist_file;
    let checklist = (!checklist_file.is_empty() && Path::new(&cwd).join(&checklist_file).is_file())
        .then_some(checklist_file);
    let inputs = Inputs {
        checklist,
        repo: RepoState::read(&cwd),
        source,
        cwd: cwd.clone(),
        task,
        mode: Some(mode),
        transcript,
        screen,
    };
    let notes = build_notes(&inputs);

    let folder = home.join(".cache").join("orcha").join("handoff");
    let path = save_notes(&folder, target, &notes, time::OffsetDateTime::now_utc())
        .map_err(|err| format!("could not save the notes: {err}"))?;

    let env = claude_dir
        .filter(|_| target == "claude")
        .map(|dir| format!("CLAUDE_CONFIG_DIR={dir}"));
    let mut split_args = vec![
        "pane",
        "split",
        "--pane",
        pane,
        "--direction",
        "right",
        "--cwd",
        &cwd,
    ];
    if let Some(env) = env.as_deref() {
        split_args.extend(["--env", env]);
    }
    let split = herdr_json(&split_args).ok_or("could not open a pane next to it")?;
    let new_pane = split
        .get("pane")
        .and_then(|pane| pane.get("pane_id"))
        .and_then(Value::as_str)
        .ok_or("could not open a pane next to it")?
        .to_owned();
    let prompt = format!(
        "Read the hand-off notes in {}, sum up where the work stands, then wait for me before changing anything.",
        path.display()
    );
    // double quotes read the same in sh, zsh, fish, PowerShell and cmd
    let line = format!("{command} \"{}\"", prompt.replace('"', "'"));
    let ran = crate::orcha_cli::succeeded(
        crate::orcha_cli::OrchaCli::new(&["pane", "run", &new_pane, &line]).quiet(),
    );
    if !ran {
        return Err(format!("{target_name} could not be started"));
    }
    Ok((
        format!("{target_name} is summing up in the new pane"),
        new_pane,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_turns_skip_meta_tool_output_and_subagents_and_collect_edits() {
        let text = [
            r#"{"type":"user","message":{"role":"user","content":"Add dark mode"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"meta"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"On it."},{"type":"tool_use","name":"Edit","input":{"file_path":"/r/a.rs"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":"sub"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Write","input":{"file_path":"/r/b.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/r/a.rs"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            "not json",
        ]
        .join("\n");
        let parsed = parse_claude(&text);
        assert_eq!(
            parsed.turns,
            vec![
                (Role::User, "Add dark mode".to_owned()),
                (Role::Agent, "On it.".to_owned()),
            ]
        );
        assert_eq!(parsed.files, vec!["/r/a.rs", "/r/b.rs"]);
    }

    #[test]
    fn codex_turns_and_patched_files() {
        let text = [
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the login"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/login.rs\n@@\n*** Add File: src/new.rs\n*** End Patch"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixed."}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
        ]
        .join("\n");
        let parsed = parse_codex(&text);
        assert_eq!(
            parsed.turns,
            vec![
                (Role::User, "Fix the login".to_owned()),
                (Role::Agent, "Fixed.".to_owned()),
            ]
        );
        assert_eq!(parsed.files, vec!["src/new.rs", "src/login.rs"]);
    }

    fn inputs() -> Inputs {
        Inputs {
            source: "claude".to_owned(),
            cwd: "/r".to_owned(),
            task: "Dark mode".to_owned(),
            mode: Some(Mode::Full),
            transcript: None,
            screen: String::new(),
            checklist: None,
            repo: RepoState {
                branch: "feature/dark".to_owned(),
                status: " M src/a.rs".to_owned(),
                diff_stat: " src/a.rs | 2 +-".to_owned(),
                commits: "abc123 start dark mode".to_owned(),
            },
        }
    }

    #[test]
    fn notes_point_at_the_checklist_only_when_one_was_left() {
        assert!(!build_notes(&inputs()).contains("checklist"));
        let mut with = inputs();
        with.checklist = Some(".orcha/handoff.md".to_owned());
        assert!(build_notes(&with).contains("`.orcha/handoff.md`"));
    }

    #[test]
    fn notes_carry_goal_latest_request_files_and_repository() {
        let mut inputs = inputs();
        inputs.transcript = Some(Transcript {
            turns: vec![
                (Role::User, "Add dark mode".to_owned()),
                (Role::Agent, "Done with the menu.".to_owned()),
                (Role::User, "Now the settings page".to_owned()),
            ],
            files: vec!["/r/src/a.rs".to_owned()],
        });
        let notes = build_notes(&inputs);
        assert!(notes.starts_with("# Hand-off: Dark mode"));
        assert!(notes.contains("another coding agent (Claude)"));
        assert!(notes.contains("- branch: `feature/dark`"));
        assert!(notes.contains("abc123 start dark mode"));
        assert!(notes.contains("## Files Claude changed (latest first)\n\n- `/r/src/a.rs`"));
        assert!(notes.contains("## The goal (first request)\n\nAdd dark mode"));
        assert!(notes.contains("## The latest request\n\nNow the settings page"));
        assert!(notes.contains("**Claude:** Done with the menu."));
    }

    #[test]
    fn notes_fall_back_to_the_screen_then_to_the_repository() {
        let mut with_screen = inputs();
        with_screen.source = "cursor".to_owned();
        with_screen.screen = "\n  I updated the header.\n".to_owned();
        let notes = build_notes(&with_screen);
        assert!(notes.contains("## Cursor's screen (its latest output)"));
        assert!(notes.contains("I updated the header."));

        let notes = build_notes(&inputs());
        assert!(notes.contains("could not be read; sum up from the repository"));
    }

    #[test]
    fn every_hand_off_asks_for_a_summary_and_to_wait() {
        for mode in [Mode::Full, Mode::Summary] {
            let mut inputs = inputs();
            inputs.mode = Some(mode);
            let notes = build_notes(&inputs);
            assert!(notes.contains("sum up where the work stands"), "{mode:?}");
            assert!(
                notes.contains("**Do not continue the work yet.**"),
                "{mode:?}"
            );
            assert!(!notes.contains("then continue"), "{mode:?}");
        }
    }

    #[test]
    fn summary_mode_cuts_long_conversations() {
        let mut inputs = inputs();
        inputs.mode = Some(Mode::Summary);
        let long = "é".repeat(SUMMARY_CHARS);
        inputs.transcript = Some(Transcript {
            turns: vec![(Role::User, "goal".to_owned()), (Role::Agent, long)],
            files: Vec::new(),
        });
        let notes = build_notes(&inputs);
        assert!(notes.contains("…(earlier turns cut)…"));
        assert!(notes.len() < SUMMARY_CHARS + 2_000);
    }

    #[test]
    fn cuts_keep_char_boundaries() {
        assert_eq!(tail("héllo", 4), "llo");
        assert_eq!(head("héllo", 2), "h");
        assert_eq!(head("hi", 10), "hi");
    }

    #[test]
    fn home_relative_folders_expand() {
        use crate::orcha_accounts::expand_home;
        let home = Path::new("/home/me");
        assert_eq!(expand_home("~/x", home), PathBuf::from("/home/me/x"));
        assert_eq!(expand_home("~", home), PathBuf::from("/home/me"));
        assert_eq!(expand_home("/abs", home), PathBuf::from("/abs"));
    }

    #[test]
    fn hand_off_targets_come_from_the_account_registry() {
        for kind in ["claude", "codex", "cursor"] {
            assert!(can_hand_off_to(kind), "{kind}");
        }
        assert!(!can_hand_off_to("gemini"));
        assert_eq!(agent_name("codex"), "Codex");
    }

    #[cfg(unix)]
    #[test]
    fn legacy_notes_become_private_without_touching_other_files_or_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("orcha-notes-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("handoff");
        std::fs::create_dir_all(&folder).unwrap();
        let note = folder.join("20261102-134903-codex.md");
        let other = folder.join("20261102-important.md");
        let outside = root.join("outside.md");
        for path in [&note, &other, &outside] {
            std::fs::write(path, "kept").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = folder.join("20261102-134903-claude.md");
        symlink(&outside, &link).unwrap();
        let now = time::OffsetDateTime::now_utc();
        save_notes(&folder, "codex", "new", now).expect("migrated");
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&note), 0o600);
        assert_eq!(mode(&other), 0o644);
        assert_eq!(mode(&outside), 0o644);
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "kept");
        // A legacy symlink directory must not redirect a new conversation.
        let redirected = root.join("redirected");
        symlink(&folder, &redirected).unwrap();
        assert!(save_notes(&redirected, "codex", "not written", now).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn notes_are_owner_only_and_old_ones_are_pruned() {
        let root = std::env::temp_dir().join(format!("orcha-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("handoff");
        let now = time::OffsetDateTime::now_utc();
        let first = save_notes(&folder, "codex", "chat", now).expect("saved");
        let second = save_notes(&folder, "codex", "chat 2", now).expect("same second");
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "chat 2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&first), 0o600);
            assert_eq!(mode(&folder), 0o700);
        }
        // a three-week-old note goes, a fresh one and other files stay
        let old = folder.join("20200101-000000-claude.md");
        let other = folder.join("keep-me.md");
        for path in [&old, &other] {
            std::fs::write(path, "x").unwrap();
            let file = std::fs::File::options().write(true).open(path).unwrap();
            let three_weeks = std::time::Duration::from_secs(21 * 24 * 60 * 60);
            file.set_modified(std::time::SystemTime::now() - three_weeks)
                .unwrap();
        }
        save_notes(&folder, "claude", "new", now).expect("saved");
        assert!(!old.exists());
        assert!(other.exists() && first.exists() && second.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_plain_session_ids_pass() {
        assert!(plain_session_id("0b46c1e2-77aa-4f00-9c1d-2e3f4a5b6c7d"));
        assert!(plain_session_id("rollout_2026-01"));
        for odd in ["", "../../secret", "a/b", "a\\b", "x; rm -rf ~", "a b", "é"] {
            assert!(!plain_session_id(odd), "{odd:?}");
        }
    }

    #[test]
    fn a_session_id_cannot_reach_a_file_outside_the_projects() {
        let home = std::env::temp_dir().join(format!("orcha-handoff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let project = home.join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&project).expect("create project");
        let chat = r#"{"type":"user","message":{"role":"user","content":"Add dark mode"}}"#;
        std::fs::write(project.join("abc-123.jsonl"), chat).expect("write chat");
        // projects/p/../../secret.jsonl is ~/.claude/secret.jsonl
        std::fs::write(home.join(".claude").join("secret.jsonl"), chat).expect("write secret");
        assert!(transcript("claude", "abc-123", &home).is_some());
        assert!(transcript("claude", "../../secret", &home).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}
