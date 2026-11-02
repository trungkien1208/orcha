use super::*;

/// An agent sign-in on this machine that orcha does not list yet: a
/// Claude config folder, or another agent's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoundAccount {
    /// herdr's agent id (`AGENTS`).
    pub kind: String,
    /// Claude's `CLAUDE_CONFIG_DIR`; None for the default folder.
    pub dir: Option<String>,
    /// Who it is signed in as; empty when the CLI does not say.
    pub email: String,
    /// The name it gets (Claude: `claude-<name>`).
    pub name: String,
    /// orcha saw its sign-in (false: installed, sign in after importing).
    pub signed_in: bool,
}

impl FoundAccount {
    /// How config.json remembers a dismissed one.
    pub(super) fn key(&self) -> String {
        if self.kind != "claude" {
            return format!("agent:{}", self.kind);
        }
        self.dir.clone().unwrap_or_else(|| "default".to_owned())
    }
}

/// Folders where Claude sign-ins usually live: orcha's own, `~/.claude-*`
/// and `CLAUDE_CONFIG_DIR`.
pub(super) fn claude_folder_candidates() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let Some(home) = home() else {
        return folders;
    };
    let mut push_children = |parent: &Path, prefix: &str| {
        let Ok(read) = std::fs::read_dir(parent) else {
            return;
        };
        let mut found: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        found.sort();
        folders.extend(found);
    };
    push_children(&home.join(".ai-accounts").join("claude"), "");
    push_children(&home, ".claude-");
    push_children(&home, ".claude_");
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            folders.push(PathBuf::from(expand(dir.trim())));
        }
    }
    folders
}

pub(super) fn same_folder(a: &str, b: &str) -> bool {
    let canonical =
        |path: &str| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    a == b || canonical(a) == canonical(b)
}

/// The name a found folder gets: its folder name without `.claude-`.
pub(super) fn found_name(dir: Option<&str>, email: &str) -> String {
    let from_folder = dir
        .and_then(|dir| Path::new(dir).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .map(|name| {
            name.trim_start_matches(".claude-")
                .trim_start_matches(".claude_")
                .trim_start_matches('.')
                .to_owned()
        })
        .filter(|name| !account_slug(name).is_empty() && name != "claude");
    from_folder.unwrap_or_else(|| {
        email
            .split('@')
            .next()
            .filter(|user| !account_slug(user).is_empty())
            .unwrap_or("main")
            .to_owned()
    })
}

/// Signed-in Claude folders and installed agents orcha does not list, in
/// the order found. `agents` are (kind, email, signed in) of the agent CLIs
/// installed. `with_dismissed` also returns the ones the user said no to.
pub(super) fn find_unlisted_in(
    config: Option<&Value>,
    candidates: &[PathBuf],
    default_email: Option<String>,
    agents: &[(&str, Option<String>, bool)],
    with_dismissed: bool,
) -> Vec<FoundAccount> {
    let listed = config.map(parse_accounts).unwrap_or_default();
    let dismissed: Vec<String> = if with_dismissed {
        Vec::new()
    } else {
        config
            .and_then(|config| config.get("dismissed_account_dirs"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|dir| dir.as_str().map(str::to_owned))
            .collect()
    };
    let mut found: Vec<FoundAccount> = Vec::new();
    // the default folder, when the accounts orcha lists all live elsewhere
    // (with none listed, `accounts` shows it already)
    let default_listed = listed
        .iter()
        .any(|account| account.kind == "claude" && account.dir.is_none());
    let any_claude = listed.iter().any(|account| account.kind == "claude");
    if let Some(email) = default_email.filter(|_| any_claude && !default_listed) {
        found.push(FoundAccount {
            kind: "claude".to_owned(),
            dir: None,
            name: found_name(None, &email),
            email,
            signed_in: true,
        });
    }
    for folder in candidates {
        let dir = folder.to_string_lossy().into_owned();
        let Some(email) = claude_folder_email(Some(&dir)) else {
            continue;
        };
        let known = listed.iter().any(|account| {
            account
                .dir
                .as_deref()
                .is_some_and(|listed_dir| same_folder(listed_dir, &dir))
        }) || found.iter().any(|other| {
            other
                .dir
                .as_deref()
                .is_some_and(|other| same_folder(other, &dir))
        });
        if known {
            continue;
        }
        found.push(FoundAccount {
            kind: "claude".to_owned(),
            name: found_name(Some(&dir), &email),
            dir: Some(dir),
            email,
            signed_in: true,
        });
    }
    // agents with fuel show by themselves (`accounts`); the others once
    // the user says so. Ones taken off the list stay off.
    let hidden = hidden_accounts(config);
    for (kind, email, signed_in) in agents {
        let Some(agent) = agent_kind(kind).filter(|agent| !agent.fuel) else {
            continue;
        };
        if listed.iter().any(|account| account.kind == *kind) || hidden.iter().any(|id| id == kind)
        {
            continue;
        }
        found.push(FoundAccount {
            kind: (*kind).to_owned(),
            dir: None,
            email: email.clone().unwrap_or_default(),
            name: agent.name.to_owned(),
            signed_in: *signed_in,
        });
    }
    found.retain(|account| {
        !dismissed
            .iter()
            .any(|key| *key == account.key() || same_folder(key, &account.key()))
    });
    found
}

/// Signed-in Claude folders and installed agents orcha does not list yet.
/// `asked`: from the
/// menu, which also brings back dismissed ones; the look orcha makes by
/// itself only runs once there is an accounts list to add to (before that,
/// `accounts` shows the default sign-ins).
pub(crate) fn find_unlisted(asked: bool) -> Vec<FoundAccount> {
    let config = config_path().and_then(|path| read_json(&path));
    if !asked
        && config
            .as_ref()
            .is_none_or(|config| config.get("accounts").is_none())
    {
        return Vec::new();
    }
    let agents: Vec<(&str, Option<String>, bool)> = match home() {
        Some(home) => AGENTS
            .iter()
            .filter(|agent| !agent.fuel && crate::orcha_handoff::command_installed(agent.command))
            .map(|agent| {
                (
                    agent.kind,
                    agent_email(&home, agent.kind),
                    agent_signed_in(&home, agent.kind),
                )
            })
            .collect(),
        None => Vec::new(),
    };
    find_unlisted_in(
        config.as_ref(),
        &claude_folder_candidates(),
        claude_folder_email(None),
        &agents,
        asked,
    )
}

/// Lists the detected accounts in config.json the first time it changes,
/// so they stay.
pub(super) fn seed_detected(config: &mut Value) {
    if config.get("accounts").is_some() {
        return;
    }
    let detected: Vec<Value> = accounts()
        .into_iter()
        .map(|account| {
            serde_json::json!({
                "id": account.id,
                "kind": account.kind,
                "label": account.label,
                "email": account.email,
                "dir": account.dir,
            })
        })
        .collect();
    if let Some(object) = config.as_object_mut() {
        object.insert("accounts".to_owned(), Value::Array(detected));
    }
}

/// Adds found accounts to config.json (their sign-ins are used as they
/// are) and drops the others from asking again. Returns the ids added.
pub(super) fn import_into_config(
    config: &mut Value,
    chosen: &[FoundAccount],
    dismissed: &[FoundAccount],
) -> Result<Vec<String>, String> {
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let list = object
        .entry("accounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("accounts in config.json is not a list")?;
    let mut added = Vec::new();
    for account in chosen {
        let taken = |id: &str| {
            list.iter()
                .any(|known| known.get("id").and_then(Value::as_str) == Some(id))
        };
        // Claude accounts are claude-<name>; another agent is its kind
        let (prefix, base) = if account.kind == "claude" {
            let base = account_slug(&account.name);
            (
                "claude-",
                if base.is_empty() {
                    "main".to_owned()
                } else {
                    base
                },
            )
        } else {
            ("", account.kind.clone())
        };
        let mut slug = base.clone();
        let mut n = 2;
        while taken(&format!("{prefix}{slug}")) {
            slug = format!("{base}{n}");
            n += 1;
        }
        let id = format!("{prefix}{slug}");
        let label = if account.kind == "claude" {
            format!("Claude · {slug}")
        } else {
            account.name.clone()
        };
        list.push(serde_json::json!({
            "id": id,
            "kind": account.kind,
            "label": label,
            "email": account.email,
            "dir": account.dir,
        }));
        added.push(id);
    }
    if !dismissed.is_empty() {
        let keys = object
            .entry("dismissed_account_dirs")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("dismissed_account_dirs in config.json is not a list")?;
        for account in dismissed {
            let key = account.key();
            if !keys
                .iter()
                .any(|known| known.as_str() == Some(key.as_str()))
            {
                keys.push(Value::String(key));
            }
        }
    }
    Ok(added)
}

/// Imports the `chosen` found accounts; the `dismissed` ones are not
/// offered again unless asked for.
pub(crate) fn import_found(
    chosen: &[FoundAccount],
    dismissed: &[FoundAccount],
) -> Result<Vec<String>, String> {
    let path = config_path().ok_or("no config folder")?;
    edit_config(&path, |config| {
        seed_detected(config);
        import_into_config(config, chosen, dismissed)
    })
}

/// NEW: a tab in `workspace` running the account's agent.
pub(crate) fn new_agent(id: &str, workspace: &str, cwd: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind == "claude" {
        // one Claude sign-in for every default session: this account becomes it
        activate(id)?;
    }
    let mut args = vec![
        "tab",
        "create",
        "--workspace",
        workspace,
        "--label",
        &account.label,
        "--focus",
    ];
    if !cwd.is_empty() {
        args.extend(["--cwd", cwd]);
    }
    let result = crate::orcha_handoff::herdr_json(&args).ok_or("could not open a tab")?;
    let pane = first_pane_id(&result).ok_or("could not open a tab")?;
    let command = agent_kind(&account.kind).map_or("claude", |agent| agent.command);
    run_in_pane(&pane, command)?;
    select(id)
}

/// The first pane id anywhere in a CLI result.
pub(super) fn first_pane_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get("pane_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| map.values().find_map(first_pane_id)),
        Value::Array(items) => items.iter().find_map(first_pane_id),
        _ => None,
    }
}
