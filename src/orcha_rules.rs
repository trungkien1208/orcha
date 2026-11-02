//! Working rules and suggested skills for a project. Rules are written as
//! one marked block in the project's CLAUDE.md (or AGENTS.md when CLAUDE.md
//! points there), so turning one on or off never touches what the team wrote
//! around it. Nothing is written or installed without the user's click.

use std::path::{Path, PathBuf};

use serde_json::Value;

const BLOCK_START: &str =
    "<!-- orcha:rules start · managed by orcha (Settings › Projects); edit them there -->";
const BLOCK_START_PREFIX: &str = "<!-- orcha:rules start";
const BLOCK_END: &str = "<!-- orcha:rules end -->";
const RULE_MARK: &str = "<!-- orcha:rule ";

pub(crate) struct Rule {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

pub(crate) const RULES: &[Rule] = &[
    Rule {
        id: "slices",
        title: "Plan in slices first",
        summary: "split the work into small slices and a todo list, wait for a go, then do one at a time",
        body: "Before writing code for anything bigger than a one-line fix:\n\
1. Split the work into thin vertical slices, each one working and testable on its own.\n\
2. Write them as a todo list (use your todo tool if you have one), with how each slice will be checked.\n\
3. Show the list and wait for a go-ahead.\n\
4. Do one slice at a time: build it, check it, mark it done, then start the next.\n\
If a slice turns out bigger than planned, stop and slice it again.",
    },
    Rule {
        id: "verify",
        title: "Verify before saying done",
        summary: "run the project's checks and report what really happened",
        body: "Do not call work done until you have run the project's own checks (typecheck, \
tests, lint) on what you changed. Report the real result: paste failures, and say plainly \
when a check was skipped or could not run.",
    },
    Rule {
        id: "commit",
        title: "Propose the commit message first",
        summary: "small commits, a message the user agrees to before committing, never push unasked",
        body: "Keep commits small and about one thing. Before committing, show the commit \
message and wait for agreement. Never push, force-push or open a pull request unless asked.",
    },
    Rule {
        id: "careful",
        title: "Ask before anything hard to undo",
        summary: "deleting files, resetting, rewriting history or touching shared systems",
        body: "Ask first before anything hard to undo: deleting or overwriting files you did not \
create, `git reset --hard`, rebases and force pushes, dropping data, or changing shared \
systems (CI, production, other people's branches).",
    },
    Rule {
        id: "scope",
        title: "Stay in scope",
        summary: "change only what the task needs; note the rest instead of fixing it",
        body: "Change only what the task needs. When you notice something else worth fixing, \
write it down in your reply instead of fixing it, so the change stays easy to review.",
    },
    Rule {
        id: "comments",
        title: "Comments say why",
        summary: "no comments that repeat the code; explain the reason or the trap",
        body: "Write comments only where the code cannot say it: why it is done this way, a \
trap, a constraint from outside. Never comments that repeat what the code does.",
    },
    Rule {
        id: "handoff",
        title: "Keep a hand-off checklist",
        summary: "on long tasks, keep .orcha/handoff.md current so another agent can continue",
        body: "On a task that takes more than a few steps, keep `.orcha/handoff.md` up to date: \
what is done, what is next as `- [ ]` items, open questions, and the files touched. Another \
agent may have to continue from it if you run out of plan.",
    },
];

/// The rules a new project gets when nothing is chosen yet.
const FIRST_DEFAULTS: &[&str] = &["slices", "verify", "commit"];

pub(crate) struct Skill {
    pub name: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub group: &'static str,
    /// GitHub `owner/repo` the skills CLI installs from.
    pub source: &'static str,
    /// Skill names inside it; the first one shows whether it is installed.
    pub skills: &'static [&'static str],
}

pub(crate) const SKILLS: &[Skill] = &[
    Skill {
        name: "to-tickets",
        title: "To tickets",
        summary: "turns a plan into small vertical slices you can pick up one by one",
        group: "Plan",
        source: "mattpocock/skills",
        skills: &["to-tickets"],
    },
    Skill {
        name: "grill-me",
        title: "Grill me",
        summary: "interviews you about a plan until every open question is answered",
        group: "Plan",
        source: "mattpocock/skills",
        skills: &["grill-me"],
    },
    Skill {
        name: "writing-plans",
        title: "Superpowers: plans",
        summary: "brainstorm, write a step-by-step plan, then execute it with checks",
        group: "Plan",
        source: "obra/superpowers",
        skills: &["writing-plans", "executing-plans", "brainstorming"],
    },
    Skill {
        name: "tdd",
        title: "Test-driven development",
        summary: "a failing test first, then the code that makes it pass",
        group: "Build",
        source: "mattpocock/skills",
        skills: &["tdd"],
    },
    Skill {
        name: "systematic-debugging",
        title: "Superpowers: debugging",
        summary: "find the cause before changing code, and verify before calling it done",
        group: "Build",
        source: "obra/superpowers",
        skills: &["systematic-debugging", "verification-before-completion"],
    },
    Skill {
        name: "ponytail",
        title: "Ponytail",
        summary: "the smallest change that solves it, plus a plain-English review",
        group: "Build",
        source: "DietrichGebert/ponytail",
        skills: &["ponytail", "ponytail-review"],
    },
    Skill {
        name: "make-interfaces-feel-better",
        title: "Interfaces that feel better",
        summary: "UI polish: spacing, motion, shadows, hover and focus details",
        group: "UI",
        source: "jakubkrehel/make-interfaces-feel-better",
        skills: &["make-interfaces-feel-better"],
    },
    Skill {
        name: "design-taste-frontend",
        title: "Design taste",
        summary: "landing pages and redesigns that do not look templated",
        group: "UI",
        source: "leonxlnx/taste-skill",
        skills: &["design-taste-frontend"],
    },
    Skill {
        name: "humanizer",
        title: "Humanizer",
        summary: "rewrites text so it stops sounding like AI",
        group: "Writing",
        source: "blader/humanizer",
        skills: &["humanizer"],
    },
    Skill {
        name: "find-skills",
        title: "Find skills",
        summary: "searches for more skills when you ask how to do something",
        group: "Writing",
        source: "vercel-labs/skills",
        skills: &["find-skills"],
    },
];

// ---- where the rules go ----

/// CLAUDE.md, unless the project keeps its rules in AGENTS.md (CLAUDE.md
/// missing, or only pointing there).
pub(crate) fn instruction_file(project: &Path) -> PathBuf {
    let claude = project.join("CLAUDE.md");
    let agents = project.join("AGENTS.md");
    if agents.is_file() {
        let points_there = std::fs::read_to_string(&claude)
            .map(|text| text.contains("AGENTS.md"))
            .unwrap_or(true);
        if points_there {
            return agents;
        }
    }
    claude
}

/// The rule ids inside the managed block of `text`.
pub(crate) fn applied_in(text: &str) -> Vec<String> {
    let Some((start, end)) = block_span(text) else {
        return Vec::new();
    };
    text[start..end]
        .match_indices(RULE_MARK)
        .filter_map(|(at, _)| {
            let rest = &text[start + at + RULE_MARK.len()..end];
            let id = rest.split_whitespace().next()?;
            RULES
                .iter()
                .any(|rule| rule.id == id)
                .then(|| id.to_owned())
        })
        .collect()
}

/// Byte range of the managed block, end marker included.
fn block_span(text: &str) -> Option<(usize, usize)> {
    let start = text.find(BLOCK_START_PREFIX)?;
    let end = start + text[start..].find(BLOCK_END)? + BLOCK_END.len();
    Some((start, end))
}

fn block_for(ids: &[String]) -> String {
    let mut out = format!("{BLOCK_START}\n## Working rules\n");
    for rule in RULES
        .iter()
        .filter(|rule| ids.iter().any(|id| id == rule.id))
    {
        out.push_str(&format!(
            "\n{RULE_MARK}{} -->\n### {}\n\n{}\n",
            rule.id, rule.title, rule.body
        ));
    }
    out.push_str(BLOCK_END);
    out
}

/// `text` with the managed block set to `ids` (removed when empty).
pub(crate) fn with_rules(text: &str, ids: &[String]) -> String {
    let block = (!ids.is_empty()).then(|| block_for(ids));
    match (block_span(text), block) {
        (Some((start, end)), Some(block)) => format!("{}{block}{}", &text[..start], &text[end..]),
        (Some((start, end)), None) => {
            let before = text[..start].trim_end_matches('\n');
            let after = text[end..].trim_start_matches('\n');
            match (before.is_empty(), after.is_empty()) {
                (true, _) => after.to_owned(),
                (false, true) => format!("{before}\n"),
                (false, false) => format!("{before}\n\n{after}"),
            }
        }
        (None, Some(block)) if text.trim().is_empty() => format!("{block}\n"),
        (None, Some(block)) => format!("{}\n\n{block}\n", text.trim_end_matches('\n')),
        (None, None) => text.to_owned(),
    }
}

/// Lines removed (-) and added (+) between two texts, with a little context.
pub(crate) fn diff(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let context = 2;
    let mut out = Vec::new();
    let from = prefix.saturating_sub(context);
    if from > 0 {
        out.push(format!("@@ line {} @@", from + 1));
    }
    out.extend(old[from..prefix].iter().map(|line| format!("  {line}")));
    out.extend(
        old[prefix..old.len() - suffix]
            .iter()
            .map(|line| format!("- {line}")),
    );
    out.extend(
        new[prefix..new.len() - suffix]
            .iter()
            .map(|line| format!("+ {line}")),
    );
    let tail = old.len() - suffix;
    out.extend(
        old[tail..(tail + context).min(old.len())]
            .iter()
            .map(|line| format!("  {line}")),
    );
    out.join("\n")
}

// ---- remembered choices ----

fn choices_path() -> PathBuf {
    crate::config::config_dir().join("project-rules.json")
}

/// What the user chose about project rules (`project-rules.json`).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Choices {
    /// The rules new projects are offered; None until changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    defaults: Option<Vec<String>>,
    /// Folders never to offer rules for again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    declined: Vec<String>,
    /// Anything else in the file, kept as it is.
    #[serde(flatten)]
    other: serde_json::Map<String, Value>,
}

fn choices() -> Choices {
    crate::orcha_state::load_or_default(&choices_path())
}

/// The rules new projects are offered.
pub(crate) fn defaults() -> Vec<String> {
    choices()
        .defaults
        .unwrap_or_else(|| FIRST_DEFAULTS.iter().map(|id| (*id).to_owned()).collect())
}

pub(crate) fn set_defaults(ids: &[String]) -> Result<(), String> {
    let ids = known(ids)?;
    crate::orcha_state::update(&choices_path(), |choices: &mut Choices| {
        choices.defaults = Some(ids);
    })
}

/// Never offer rules for this folder again.
pub(crate) fn decline(folder: &Path) -> Result<(), String> {
    let key = folder.to_string_lossy().into_owned();
    crate::orcha_state::update(&choices_path(), |choices: &mut Choices| {
        if !choices.declined.contains(&key) {
            choices.declined.push(key);
        }
    })
}

fn known(ids: &[String]) -> Result<Vec<String>, String> {
    for id in ids {
        if !RULES.iter().any(|rule| rule.id == id) {
            return Err(format!("unknown rule: {id}"));
        }
    }
    Ok(RULES
        .iter()
        .filter(|rule| ids.iter().any(|id| id == rule.id))
        .map(|rule| rule.id.to_owned())
        .collect())
}

/// Whether a freshly added project should be asked about rules: it has no
/// orcha block yet, was not declined, and there are defaults to offer.
/// Blocking (two small reads): call it off hot paths.
pub(crate) fn should_offer(folder: &Path) -> bool {
    let declined = choices()
        .declined
        .iter()
        .any(|entry| *entry == folder.to_string_lossy());
    !declined
        && folder.is_dir()
        && !defaults().is_empty()
        && applied_in(&std::fs::read_to_string(instruction_file(folder)).unwrap_or_default())
            .is_empty()
}

/// The team's instruction file as it is: empty when there is none yet, an
/// error when it cannot be read (so it is never written over).
fn read_instructions(file: &Path) -> Result<String, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(format!("could not read {}: {err}", file.display())),
    }
}

/// What applying `ids` to `project` would change: (file, diff, changed).
pub(crate) fn preview(project: &Path, ids: &[String]) -> Result<(PathBuf, String, bool), String> {
    let ids = known(ids)?;
    let file = instruction_file(project);
    let old = read_instructions(&file)?;
    let new = with_rules(&old, &ids);
    Ok((file, diff(&old, &new), old != new))
}

pub(crate) fn apply(project: &Path, ids: &[String]) -> Result<PathBuf, String> {
    let ids = known(ids)?;
    let file = instruction_file(project);
    let old = read_instructions(&file)?;
    let new = with_rules(&old, &ids);
    if new != old {
        // all or nothing: a failed write leaves the team's file whole
        crate::orcha_app::write_atomic(&file, &new)
            .map_err(|err| format!("could not write {}: {err}", file.display()))?;
    }
    tracing::info!(file = %file.display(), rules = ?ids, "orcha: project rules written");
    Ok(file)
}

// ---- skills ----

/// Where a skill is installed: (in this project, for this user).
pub(crate) fn skill_installed(project: &Path, name: &str) -> (bool, bool) {
    let found = |root: &Path| {
        root.join(".claude")
            .join("skills")
            .join(name)
            .join("SKILL.md")
            .is_file()
    };
    let global = crate::platform::user_home_dir().is_some_and(|home| found(&home));
    (found(project), global)
}

/// Installs one suggested skill with the skills CLI, in the project or for
/// the user. Blocking (downloads from GitHub): call it off the input path.
pub(crate) fn install_skill(project: &Path, name: &str, global: bool) -> Result<String, String> {
    let skill = SKILLS
        .iter()
        .find(|skill| skill.name == name)
        .ok_or_else(|| format!("unknown skill: {name}"))?;
    // Node installs npx as a .cmd script on Windows, which Command does not find by its bare name
    let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
    let mut command = crate::noninteractive_process::command(npx);
    command
        .args(["-y", "skills", "add", skill.source, "--skill"])
        .args(skill.skills)
        .args(["-a", "claude-code", "-y"]);
    if global {
        command.arg("-g");
    }
    let output = command
        .current_dir(project)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| format!("could not run npx (is Node.js installed?): {err}"))?;
    tracing::info!(
        skill = name,
        global,
        ok = output.status.success(),
        "orcha: skill install"
    );
    if output.status.success() {
        Ok(format!("{} installed", skill.title))
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        let last = text
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("the install failed");
        Err(last.trim().to_owned())
    }
}

// ---- which projects ----

/// The folder a set of working directories belongs to: their git root.
fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// orcha's projects as (name, folder): each workspace's git root, found
/// from where its panes run. Blocking (API calls): off the input path.
pub(crate) fn projects() -> Vec<(String, PathBuf)> {
    let call = |method: &str| crate::orcha_handoff::herdr_json(&["api", "call", method]);
    let labels: std::collections::HashMap<String, String> = call("workspace.list")
        .and_then(|result| result["workspaces"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|workspace| {
            Some((
                workspace["workspace_id"].as_str()?.to_owned(),
                workspace["label"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let home = crate::platform::user_home_dir();
    let mut seen: Vec<(String, PathBuf)> = Vec::new();
    for pane in call("pane.list")
        .and_then(|result| result["panes"].as_array().cloned())
        .unwrap_or_default()
    {
        let Some(cwd) = pane["cwd"].as_str().filter(|cwd| !cwd.is_empty()) else {
            continue;
        };
        let root = project_root(Path::new(cwd));
        if home.as_deref() == Some(root.as_path()) || seen.iter().any(|(_, path)| *path == root) {
            continue;
        }
        let folder = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = pane["workspace_id"]
            .as_str()
            .and_then(|id| labels.get(id))
            .map(|label| {
                label
                    .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
                    .trim()
                    .to_owned()
            })
            .filter(|label| !label.is_empty())
            .unwrap_or(folder);
        seen.push((label, root));
    }
    seen.sort_by_key(|(name, _)| name.to_lowercase());
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_block_goes_after_what_the_team_wrote_and_comes_back_out_cleanly() {
        let mine = "# Team rules\n\nUse tabs.\n";
        let with = with_rules(mine, &ids(&["slices", "verify"]));
        assert!(with.starts_with(mine.trim_end()));
        assert_eq!(applied_in(&with), ids(&["slices", "verify"]));
        // changing the set rewrites only the block
        let fewer = with_rules(&with, &ids(&["verify"]));
        assert_eq!(applied_in(&fewer), ids(&["verify"]));
        assert!(fewer.contains("Use tabs."));
        // none left: the file is as it was
        assert_eq!(with_rules(&fewer, &[]), mine);
    }

    #[test]
    fn text_after_the_block_survives_a_rewrite() {
        let text = format!("top\n\n{}\n\nbottom\n", block_for(&ids(&["scope"])));
        let rewritten = with_rules(&text, &ids(&["comments"]));
        assert!(rewritten.starts_with("top\n\n"));
        assert!(rewritten.ends_with("\n\nbottom\n"));
        assert_eq!(applied_in(&rewritten), ids(&["comments"]));
    }

    #[test]
    fn unknown_rules_are_refused_and_order_follows_the_catalog() {
        assert!(known(&ids(&["nope"])).is_err());
        assert_eq!(
            known(&ids(&["verify", "slices"])).unwrap(),
            ids(&["slices", "verify"])
        );
    }

    #[test]
    fn rules_go_to_agents_md_when_claude_md_points_there() {
        let dir = std::env::temp_dir().join(format!("orcha-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(instruction_file(&dir), dir.join("CLAUDE.md"));
        std::fs::write(dir.join("AGENTS.md"), "# rules\n").unwrap();
        assert_eq!(instruction_file(&dir), dir.join("AGENTS.md"));
        std::fs::write(dir.join("CLAUDE.md"), "Read AGENTS.md first.\n").unwrap();
        assert_eq!(instruction_file(&dir), dir.join("AGENTS.md"));
        std::fs::write(dir.join("CLAUDE.md"), "Own rules here.\n").unwrap();
        assert_eq!(instruction_file(&dir), dir.join("CLAUDE.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn applying_rules_writes_through_a_symlinked_claude_md() {
        let dir = std::env::temp_dir().join(format!("orcha-rules-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shared")).unwrap();
        let real = dir.join("shared").join("team.md");
        std::fs::write(&real, "# Team rules\n\nUse tabs.\n").unwrap();
        let link = dir.join("CLAUDE.md");
        std::os::unix::fs::symlink("shared/team.md", &link).unwrap();

        assert_eq!(apply(&dir, &ids(&["verify"])).unwrap(), link);

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let text = std::fs::read_to_string(&real).unwrap();
        assert!(text.starts_with("# Team rules\n\nUse tabs."));
        assert_eq!(applied_in(&text), ids(&["verify"]));
        // no temporary file left next to either
        for folder in [dir.clone(), dir.join("shared")] {
            for entry in std::fs::read_dir(folder).unwrap() {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                assert!(!name.contains("orcha-tmp"), "{name}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_diff_shows_only_what_changes() {
        let diff = diff("a\nb\nc\n", "a\nb\nX\nc\n");
        assert!(diff.contains("+ X"));
        assert!(!diff.contains("- "));
    }

    #[test]
    fn every_suggested_skill_names_a_source_and_a_skill() {
        for skill in SKILLS {
            assert!(skill.source.contains('/'), "{}", skill.name);
            assert!(!skill.skills.is_empty(), "{}", skill.name);
        }
    }
}
