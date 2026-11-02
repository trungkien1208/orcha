use super::*;

pub(super) fn home() -> Option<PathBuf> {
    crate::platform::user_home_dir()
}

/// orcha's config folder (shared with orcha-config).
pub(super) fn orcha_dir() -> Option<PathBuf> {
    Some(crate::platform::orcha_config_base_dir()?.join("orcha"))
}

pub(super) fn cache_dir() -> Option<PathBuf> {
    Some(home()?.join(".cache").join("orcha").join("limits"))
}

pub(super) fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

pub(super) fn expand(path: &str) -> String {
    let Some(home) = home() else {
        return path.to_owned();
    };
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None if path == "~" => home.to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

/// A JSON file, None when it is missing or not valid JSON. Only for reading:
/// a file orcha will write back goes through `read_json_strict`.
pub(super) fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// A JSON file orcha may write back: None when it is missing (or empty),
/// an error when it cannot be read or is not valid JSON, so the caller
/// never writes over what is in it.
pub(super) fn read_json_strict(path: &Path) -> Result<Option<Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("could not read {}: {err}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text).map(Some).map_err(|_| {
        format!(
            "{} is not valid JSON, so orcha left it as it is. Fix it and try again.",
            path.display()
        )
    })
}

pub(super) fn config_path() -> Option<PathBuf> {
    Some(orcha_dir()?.join("config.json"))
}

/// Changes orcha's config.json in place, other settings kept. A config.json
/// that cannot be read is left alone and the change is refused.
pub(super) fn edit_config<T>(
    path: &Path,
    change: impl FnOnce(&mut Value) -> Result<T, String>,
) -> Result<T, String> {
    let _lock = crate::orcha_app::lock_for_update(path)
        .map_err(|err| format!("could not lock {}: {err}", path.display()))?;
    let mut config = read_json_strict(path)?.unwrap_or_else(|| serde_json::json!({}));
    let out = change(&mut config)?;
    let text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    write_atomic(path, &format!("{text}\n")).map_err(|err| format!("could not save: {err}"))?;
    Ok(out)
}

pub(super) fn parse_accounts(config: &Value) -> Vec<AccountConfig> {
    config
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|account| {
            let field = |key: &str| account.get(key).and_then(Value::as_str).unwrap_or("");
            let id = field("id");
            let kind = field("kind");
            if id.is_empty() || agent_kind(kind).is_none() {
                return None;
            }
            Some(AccountConfig {
                id: id.to_owned(),
                kind: kind.to_owned(),
                label: if field("label").is_empty() {
                    id.to_owned()
                } else {
                    field("label").to_owned()
                },
                email: field("email").to_owned(),
                dir: Some(field("dir")).filter(|dir| !dir.is_empty()).map(expand),
            })
        })
        .collect()
}

/// The signed-in email of a Claude config folder (`~/.claude.json` for the
/// default one).
pub(super) fn claude_folder_email(dir: Option<&str>) -> Option<String> {
    let path = match dir {
        Some(dir) => Path::new(dir).join(".claude.json"),
        None => home()?.join(".claude.json"),
    };
    read_json(&path)?
        .get("oauthAccount")?
        .get("emailAddress")?
        .as_str()
        .map(str::to_owned)
}

/// The configured accounts, plus one for each of Claude, Codex and Cursor
/// installed but not listed yet.
pub(crate) fn accounts() -> Vec<AccountConfig> {
    let config = config_path().and_then(|path| read_json(&path));
    let mut out = config.as_ref().map(parse_accounts).unwrap_or_default();
    let hidden = hidden_accounts(config.as_ref());
    // the agents with fuel show by themselves; the others are offered
    // (`find_unlisted`)
    for AgentKind {
        kind,
        command,
        name,
        ..
    } in AGENTS.iter().filter(|agent| agent.fuel)
    {
        let (kind, command, name) = (*kind, *command, *name);
        let id = if kind == "claude" {
            "claude-main"
        } else {
            kind
        };
        if out.iter().any(|account| account.kind == kind)
            || hidden.iter().any(|known| known == id)
            || !crate::orcha_handoff::command_installed(command)
        {
            continue;
        }
        out.push(AccountConfig {
            id: if kind == "claude" {
                "claude-main".to_owned()
            } else {
                kind.to_owned()
            },
            kind: kind.to_owned(),
            label: name.to_owned(),
            email: if kind == "claude" {
                claude_folder_email(None).unwrap_or_default()
            } else {
                String::new()
            },
            dir: None,
        });
    }
    out
}

/// Detected accounts the user removed (`hidden_accounts` in config.json).
pub(super) fn hidden_accounts(config: Option<&Value>) -> Vec<String> {
    config
        .and_then(|config| config.get("hidden_accounts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

/// Takes `id` off orcha's list in config.json; a detected account (not in
/// the list) is hidden so it does not come back. Sign-ins and folders stay.
pub(super) fn remove_from_config(config: &mut Value, id: &str) -> Result<(), String> {
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let mut listed = false;
    if let Some(list) = object.get_mut("accounts").and_then(Value::as_array_mut) {
        let before = list.len();
        list.retain(|account| account.get("id").and_then(Value::as_str) != Some(id));
        listed = list.len() != before;
    }
    if !listed {
        let hidden = object
            .entry("hidden_accounts")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("hidden_accounts in config.json is not a list")?;
        if !hidden.iter().any(|known| known.as_str() == Some(id)) {
            hidden.push(Value::String(id.to_owned()));
        }
    }
    Ok(())
}

/// Removes an account from orcha (its sign-in and folder are kept, so
/// adding it again later just works).
pub(crate) fn remove(id: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config folder")?;
    edit_config(&path, |config| remove_from_config(config, id))?;
    // the default sign-in no longer belongs to a removed account
    if active_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .is_some_and(|active| active.trim() == id)
    {
        if let Some(file) = active_file() {
            let _ = std::fs::remove_file(file);
        }
    }
    forget_limits(id);
    let mut slots = read_string_map(slots_file());
    slots.retain(|_, owner| owner != id);
    write_string_map(slots_file(), &slots)
}

pub(super) fn find<'a>(accounts: &'a [AccountConfig], id: &str) -> Option<&'a AccountConfig> {
    accounts.iter().find(|account| account.id == id)
}
