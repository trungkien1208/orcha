use super::*;

pub(super) fn selected(accounts: &[AccountConfig]) -> Option<String> {
    let chosen = selected_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| find(accounts, id).is_some());
    chosen.or_else(|| accounts.first().map(|account| account.id.clone()))
}

/// The account new agents of `kind` use: for Claude, the one in the default
/// `~/.claude` sign-in; otherwise the only account of that kind.
pub(crate) fn default_account(accounts: &[AccountConfig], kind: &str) -> Option<String> {
    if kind == "claude" {
        return active_claude(accounts);
    }
    accounts
        .iter()
        .find(|account| account.kind == kind)
        .map(|account| account.id.clone())
}

/// Makes `id` the default of its kind. Claude: every default session moves
/// to it (`activate`).
pub(crate) fn set_default(id: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind == "claude" {
        return activate(id);
    }
    select(id)
}

/// The plan name an account is on, from its sign-in profile. Never a token.
pub(crate) fn plan(account: &AccountConfig) -> Option<String> {
    match account.kind.as_str() {
        "claude" => {
            let accounts = accounts();
            let profile = claude_profile(&accounts, &account.id).or_else(|| {
                (account.dir.is_none() || active_claude(&accounts).as_deref() == Some(&account.id))
                    .then(|| {
                        read_json(&home()?.join(".claude.json"))?
                            .get("oauthAccount")
                            .cloned()
                    })
                    .flatten()
            })?;
            let kind = profile.get("organizationType")?.as_str()?;
            Some(kind.strip_prefix("claude_").unwrap_or(kind).to_owned())
        }
        "codex" => {
            let home = crate::integration::env::codex_dir().ok()?;
            let auth = read_json(&home.join("auth.json"))?;
            let token = auth.get("tokens")?.get("id_token")?.as_str()?;
            jwt_payload(token)?
                .get("https://api.openai.com/auth")?
                .get("chatgpt_plan_type")?
                .as_str()
                .map(str::to_owned)
        }
        _ => None,
    }
}

/// Renames an account's label in config.json (other settings kept).
pub(crate) fn rename(id: &str, label: &str) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("the label is empty".to_owned());
    }
    let path = config_path().ok_or("no config folder")?;
    edit_config(&path, |config| {
        // a first run lists the detected accounts too, so they stay
        seed_detected(config);
        let entry = config
            .get_mut("accounts")
            .and_then(Value::as_array_mut)
            .and_then(|list| {
                list.iter_mut()
                    .find(|account| account.get("id").and_then(Value::as_str) == Some(id))
            })
            .ok_or("unknown account")?;
        let object = entry.as_object_mut().ok_or("account is not an object")?;
        object.insert("label".to_owned(), Value::String(label.to_owned()));
        Ok(())
    })
}

/// The account a pane's agent runs on.
pub(crate) fn account_of_pane(accounts: &[AccountConfig], pane: &str) -> Option<String> {
    pane_account(accounts, pane)
}

/// `account_of_pane` when the caller already knows the agent's kind, which
/// saves asking the server for the pane.
pub(crate) fn account_of_agent(
    accounts: &[AccountConfig],
    kind: &str,
    pane: &str,
) -> Option<String> {
    agent_account(accounts, kind, pane)
}

/// `CLAUDE_CONFIG_DIR` of the Claude in a pane; None for the default folder.
pub(crate) fn claude_folder_of_pane(pane: &str) -> Option<String> {
    pane_config_dir(pane)
}

pub(crate) fn find_account<'a>(
    accounts: &'a [AccountConfig],
    id: &str,
) -> Option<&'a AccountConfig> {
    find(accounts, id)
}

/// Prepares an extra Claude account's folder so it shares history and
/// settings with `~/.claude`, before an agent starts in it.
pub(crate) fn prepare_folder(account: &AccountConfig) -> Result<(), String> {
    match account.dir.as_deref().filter(|_| account.kind == "claude") {
        Some(dir) => link_shared(dir),
        None => Ok(()),
    }
}

/// Remembers `id` as the account picked last (selected-account).
pub(super) fn select(id: &str) -> Result<(), String> {
    let path = selected_file().ok_or("no config folder")?;
    write_atomic(&path, &format!("{id}\n"))
        .map_err(|err| format!("could not save the chosen account: {err}"))
}
