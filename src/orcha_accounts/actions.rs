use super::*;

// ---------- actions

/// Makes running Claude sessions of a folder (None: `~/.claude`) read
/// their sign-in again. With a `.credentials.json`, Claude only watches
/// that file's modified time, so touching it (content untouched) makes it
/// drop the token it holds and read the sign-in again.
pub(super) fn nudge_claude(dir: Option<&str>) {
    let folder = match dir {
        Some(dir) => PathBuf::from(expand(dir)),
        None => match home() {
            Some(home) => home.join(".claude"),
            None => return,
        },
    };
    let path = folder.join(".credentials.json");
    if let Ok(file) = std::fs::File::options().append(true).open(&path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Puts an account's profile (who is signed in) into a Claude state file
/// (`.claude.json`). A missing one is left for Claude to make.
pub(super) fn set_profile(state_path: &Path, profile: &Value) -> Result<(), String> {
    let Some(mut state) = read_json_strict(state_path)? else {
        return Ok(());
    };
    let object = state
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", state_path.display()))?;
    object.insert("oauthAccount".to_owned(), profile.clone());
    let text = serde_json::to_string_pretty(&state).map_err(|err| err.to_string())?;
    write_atomic(state_path, &text)
        .map_err(|err| format!("could not save {}: {err}", state_path.display()))
}

/// Every default Claude session moves to `id` (no restart).
pub(crate) fn activate(id: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind != "claude" {
        return Err("not a Claude account".to_owned());
    }
    let current = active_claude(&accounts);
    if current.as_deref() == Some(id) {
        return select(id);
    }
    secret_cache::invalidate_all();
    let source = crate::platform::orcha_secret_read(&kept_slot(id))
        .or_else(|| crate::platform::orcha_secret_read(&folder_slot(account)))
        .ok_or("no sign-in found: sign in")?;
    let default = OrchaSecretSlot::Folder(None);
    let home = home().ok_or("no home folder")?;
    let claude_json = home.join(".claude.json");
    if let Some(current) = &current {
        // keep the live sign-in (maybe refreshed since) and its profile first
        if let Some(live) = crate::platform::orcha_secret_read(&default) {
            secret_cache::write(&kept_slot(current), &live, &default)
                .map_err(|err| format!("could not keep the current sign-in: {err}"))?;
        }
        keep_profile(current, &claude_json)?;
    }
    // Record the new owner before the sign-in moves, and put the record back
    // if it can't: the next switch keeps whatever the record names, so a
    // record that disagrees with the sign-in would file one account's
    // sign-in under another.
    save_active(id)?;
    if let Err(err) = secret_cache::write(&default, &source, &default) {
        let undo = match &current {
            Some(current) => save_active(current),
            None => clear_active(),
        };
        if let Err(undo) = undo {
            tracing::warn!(error = %undo, "orcha: could not restore the active Claude account");
        }
        return Err(format!("could not switch the sign-in: {err}"));
    }
    forget_limits(id);
    nudge_claude(None);
    let profile = match claude_profile(&accounts, id) {
        Some(profile) => set_profile(&claude_json, &profile),
        None => Ok(()),
    };
    let selected = select(id);
    profile.and(selected)
}

/// Moves a running Claude onto `id` without a restart: the account's
/// sign-in goes into the one that pane's Claude reads. Claude reads it
/// again within about 30 seconds and the chat goes on.
pub(super) fn switch_claude_pane(
    accounts: &[AccountConfig],
    id: &str,
    pane: &str,
) -> Result<(), String> {
    let Some(dir) = pane_config_dir(pane) else {
        return activate(id);
    };
    let slot = OrchaSecretSlot::Folder(Some(dir.clone()));
    let service = crate::platform::orcha_claude_service(&slot);
    let owner = slot_owner(accounts, &service);
    if owner.as_deref() == Some(id) {
        return select(id);
    }
    // pick the source from fresh reads: a cached one may be minutes old
    secret_cache::invalidate_all();
    let source = claude_slot(accounts, id)
        .and_then(|from| crate::platform::orcha_secret_read(&from))
        .ok_or("no sign-in found: sign in")?;
    // Record the slot's new owner before its sign-in moves and put the old
    // record back if it can't: the next switch keeps the live sign-in for
    // whoever the record names.
    let before = read_slots(accounts);
    let mut slots = before.clone();
    let native = claude_accounts(accounts)
        .find(|account| crate::platform::orcha_claude_service(&folder_slot(account)) == service)
        .map(|account| account.id.clone());
    if native.as_deref() == Some(id) {
        slots.remove(&service);
    } else {
        slots.insert(service.clone(), id.to_owned());
    }
    write_string_map(slots_file(), &slots)?;
    let moved = (|| {
        if let (Some(owner), Some(live)) = (&owner, crate::platform::orcha_secret_read(&slot)) {
            // keep the sign-in being replaced (maybe refreshed since) for its account
            secret_cache::write(&kept_slot(owner), &live, &slot)
                .map_err(|err| format!("could not keep the current sign-in: {err}"))?;
        }
        secret_cache::write(&slot, &source, &slot)
            .map_err(|err| format!("could not switch the sign-in: {err}"))
    })();
    if let Err(err) = moved {
        if let Err(undo) = write_string_map(slots_file(), &before) {
            tracing::warn!(error = %undo, "orcha: could not restore the Claude slot record");
        }
        return Err(err);
    }
    // The sign-in has moved: finish every step below and report the first
    // failure, so one bad write does not leave the rest undone.
    let mut dirs = read_string_map(slot_dirs_file());
    dirs.insert(service, dir.clone());
    let dirs_saved = write_string_map(slot_dirs_file(), &dirs);
    forget_limits(id);
    nudge_claude(Some(&dir));
    // the folder's profile shows who is signed in
    let profile = match claude_profile(accounts, id) {
        Some(profile) => set_profile(&Path::new(&expand(&dir)).join(".claude.json"), &profile),
        None => Ok(()),
    };
    let selected = select(id);
    dirs_saved.and(profile).and(selected)
}

pub(super) fn run_in_pane(pane: &str, line: &str) -> Result<(), String> {
    use crate::orcha_cli::{succeeded, OrchaCli};
    if succeeded(OrchaCli::new(&["pane", "run", pane, line]).quiet()) {
        Ok(())
    } else {
        Err("could not run it in the pane".to_owned())
    }
}

pub(super) fn send_keys(pane: &str, keys: &str) {
    let _ = crate::orcha_cli::OrchaCli::new(&["pane", "send-keys", pane, keys]).quiet();
}

/// The Claude config folder holding `id`'s sign-in now (None: the default
/// `~/.claude`), or why a restarted Claude cannot run on it. Only reads.
pub(super) fn restart_folder(
    accounts: &[AccountConfig],
    id: &str,
) -> Result<Option<String>, String> {
    let account = find(accounts, id).ok_or("unknown account")?;
    if account.kind != "claude" {
        return Err("only Claude restarts on another account".to_owned());
    }
    match claude_slot(accounts, id) {
        Some(OrchaSecretSlot::Folder(dir)) => Ok(dir),
        Some(OrchaSecretSlot::Service(service)) => read_string_map(slot_dirs_file())
            .get(&service)
            .cloned()
            .map(Some)
            .ok_or_else(|| set_aside(account)),
        _ => Err(set_aside(account)),
    }
}

pub(super) fn set_aside(account: &AccountConfig) -> String {
    format!(
        "{}'s sign-in is set aside, in no folder; swap the sign-in instead",
        account.label
    )
}

/// Why `id` cannot run a restarted Claude, or None when it can.
pub(crate) fn restart_blocker(id: &str) -> Option<String> {
    restart_folder(&accounts(), id).err()
}

/// The shell line that starts Claude with its config in `dir` (None: the
/// default folder), resuming `session`.
pub(super) fn claude_restart_line(dir: Option<&str>, session: Option<&str>) -> String {
    let resume = session
        .filter(|session| crate::orcha_handoff::plain_session_id(session))
        .map(|session| format!(" --resume {session}"))
        .unwrap_or_default();
    crate::platform::orcha_env_command_line("CLAUDE_CONFIG_DIR", dir, &format!("claude{resume}"))
}

/// Stops the Claude in `pane` and starts it again on `id`, resuming the same
/// conversation. Only this pane moves; other Claude sessions keep theirs.
pub(crate) fn restart_on(id: &str, pane: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    let dir = restart_folder(&accounts, id)?;
    let session = pane_info(pane).and_then(|info| {
        let session = info.get("agent_session")?;
        session
            .get("id")
            .or_else(|| session.get("value"))?
            .as_str()
            .map(str::to_owned)
    });
    // without the shared history the resumed chat would not be found
    prepare_folder(account)?;
    // Ctrl+C twice leaves Claude; then the shell starts it on the account
    for _ in 0..2 {
        send_keys(pane, "ctrl+c");
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_in_pane(
        pane,
        &claude_restart_line(dir.as_deref(), session.as_deref()),
    )?;
    forget_limits(id);
    select(id)
}

/// The shell line that continues an agent other than Claude: Codex resumes
/// the pane's own chat when its id is plain, else its last one.
pub(super) fn resume_line(kind: &str, session: Option<&str>) -> String {
    match (kind, session) {
        ("codex", Some(session)) if crate::orcha_handoff::plain_session_id(session) => {
            format!("codex resume {session}")
        }
        (kind, _) => agent_kind(kind).map_or_else(
            || kind.to_owned(),
            |agent| agent.resume.unwrap_or(agent.command).to_owned(),
        ),
    }
}

/// Continues `pane`'s conversation on another account. Claude: the running
/// session moves without a restart. Others: stop the agent and resume the
/// same conversation there.
pub(crate) fn switch(id: &str, pane: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind == "claude" {
        return switch_claude_pane(&accounts, id, pane);
    }
    let session = pane_info(pane).and_then(|info| {
        info.get("agent_session")?
            .get("value")?
            .as_str()
            .map(str::to_owned)
    });
    let line = resume_line(&account.kind, session.as_deref());
    // Ctrl+C twice leaves the agent's CLI; then the shell resumes
    for _ in 0..2 {
        send_keys(pane, "ctrl+c");
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_in_pane(pane, &line)?;
    select(id)
}

/// An extra Claude account's folder: everything of `~/.claude` linked in
/// (history, settings, skills...) except the sign-in files, so accounts
/// share their conversations.
pub(super) fn link_shared(target: &str) -> Result<(), String> {
    let home = home().ok_or("no home folder")?;
    let source = home.join(".claude");
    let target = PathBuf::from(expand(target));
    std::fs::create_dir_all(&target)
        .map_err(|err| format!("could not make {}: {err}", target.display()))?;
    const PRIVATE: [&str; 7] = [
        ".claude.json",
        ".credentials.json",
        "daemon",
        "daemon.log",
        "daemon-auth-cooldown",
        "daemon-auth-status.json",
        "backups",
    ];
    let entries = match std::fs::read_dir(&source) {
        Ok(entries) => entries,
        // no ~/.claude yet: nothing to share
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("could not read {}: {err}", source.display())),
    };
    let mut failed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if PRIVATE.contains(&name.as_str())
            || name.starts_with("policy-limits.json")
            || name.starts_with("remote-settings.json")
            || name.starts_with("..")
            || name.contains(".tmp")
        {
            continue;
        }
        let link = target.join(&name);
        if std::fs::symlink_metadata(&link).is_err() {
            if let Err(err) = crate::platform::orcha_link_path(&entry.path(), &link) {
                failed.push(format!("{name} ({err})"));
            }
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "could not link {} into {}: {}",
            source.display(),
            target.display(),
            failed.join(", ")
        ))
    }
}

/// A new pane right of `pane` (or of the focused one), in `cwd`, with an
/// account's config folder in its environment.
pub(super) fn split_for(
    account: &AccountConfig,
    pane: Option<&str>,
    cwd: Option<&str>,
) -> Result<String, String> {
    let mut args = vec!["pane", "split", "--direction", "right"];
    match pane {
        Some(pane) => args.extend(["--pane", pane]),
        None => args.push("--current"),
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        args.extend(["--cwd", cwd]);
    }
    let env;
    if let Some(dir) = account.dir.as_deref().filter(|_| account.kind == "claude") {
        link_shared(dir)?;
        env = format!("CLAUDE_CONFIG_DIR={dir}");
        args.extend(["--env", &env]);
    }
    crate::orcha_handoff::herdr_json(&args)
        .and_then(|result| {
            result
                .get("pane")?
                .get("pane_id")?
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(|| "could not open a pane".to_owned())
}

/// The agent's command continuing its last chat, else a new one.
pub(super) fn continue_line(kind: &str) -> String {
    let Some(agent) = agent_kind(kind) else {
        return kind.to_owned();
    };
    match agent.resume {
        Some(resume) => crate::platform::orcha_fallback_command_line(resume, agent.command),
        None => agent.command.to_owned(),
    }
}

/// Opens an account in a new pane next to `pane`, continuing its last chat.
pub(crate) fn open(id: &str, pane: Option<&str>, cwd: Option<&str>) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    let new_pane = split_for(account, pane, cwd)?;
    run_in_pane(&new_pane, &continue_line(&account.kind))
}

/// An email safe to put on a command line as it is.
pub(super) fn plain_email(email: &str) -> Option<&str> {
    let email = email.trim();
    (!email.is_empty()
        && email
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "@._+-".contains(ch)))
    .then_some(email)
}

/// Signs an account in, in a new pane next to `pane`.
pub(crate) fn login(id: &str, pane: Option<&str>) -> Result<(), String> {
    secret_cache::invalidate_all();
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    // Claude writes the sign-in where this account will look for it
    prepare_sign_in(&accounts, id)?;
    let new_pane = split_for(account, pane, None)?;
    let line = match account.kind.as_str() {
        "claude" => match plain_email(&account.email) {
            Some(email) => format!("claude auth login --email {email}"),
            None => "claude auth login".to_owned(),
        },
        kind => agent_kind(kind)
            .map_or(kind, |agent| agent.login)
            .to_owned(),
    };
    forget_limits(id);
    run_in_pane(&new_pane, &line)
}

/// "Personal Mac" → "personal-mac".
pub(super) fn account_slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// Adds the Claude account `name` to config.json (other settings kept).
pub(super) fn add_to_config(config: &mut Value, name: &str) -> Result<String, String> {
    let slug = account_slug(name);
    if slug.is_empty() {
        return Err("need a name like personal or work".to_owned());
    }
    let id = format!("claude-{slug}");
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let list = object
        .entry("accounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("accounts in config.json is not a list")?;
    let taken = list.iter().any(|account| {
        let known = account.get("id").and_then(Value::as_str).unwrap_or("");
        known == id || known.strip_prefix("claude-") == Some(slug.as_str())
    });
    if taken {
        return Err(format!("{id} is already an account"));
    }
    list.push(serde_json::json!({
        "id": id,
        "kind": "claude",
        "label": format!("Claude · {slug}"),
        "email": "",
        "dir": format!("~/.ai-accounts/claude/{slug}"),
    }));
    Ok(id)
}

/// Adds a Claude account and signs it in, in a new pane next to `pane`.
pub(crate) fn add(name: &str, pane: Option<&str>) -> Result<String, String> {
    let path = config_path().ok_or("no config folder")?;
    let id = edit_config(&path, |config| {
        // a first run lists the detected accounts too, so they stay
        seed_detected(config);
        add_to_config(config, name)
    })?;
    login(&id, pane)?;
    Ok(id)
}
