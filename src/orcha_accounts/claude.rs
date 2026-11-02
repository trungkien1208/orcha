use super::*;

// ---------- where each Claude sign-in lives
//
// Every Claude session reads the sign-in of its config folder. `activate`
// puts an account's sign-in into the default folder's, so running default
// sessions move with it, no restart. The sign-in it replaces (Claude may
// have refreshed it) is kept first under `Kept(<id>)`, so that account still
// works when it comes back. A pane started in an account's own folder reads
// that folder's sign-in; switching such a pane puts the new account's
// sign-in there, and claude-slots.json remembers which account each folder
// holds now ({service: account}).

pub(super) fn folder_slot(account: &AccountConfig) -> OrchaSecretSlot {
    OrchaSecretSlot::Folder(account.dir.clone())
}

pub(super) fn kept_slot(id: &str) -> OrchaSecretSlot {
    OrchaSecretSlot::Kept(id.to_owned())
}

pub(super) fn active_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-active"))
}

pub(super) fn profile_dir() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-accounts"))
}

pub(super) fn slots_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-slots.json"))
}

/// `{service: folder}` for the slots orcha filled, so hosts that keep
/// sign-ins in files can find the folder behind a service name.
pub(super) fn slot_dirs_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-slot-dirs.json"))
}

pub(super) fn selected_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("selected-account"))
}

pub(super) fn read_string_map(path: Option<PathBuf>) -> HashMap<String, String> {
    path.and_then(|path| read_json(&path))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(super) fn write_string_map(
    path: Option<PathBuf>,
    map: &HashMap<String, String>,
) -> Result<(), String> {
    // no config folder: nothing was recorded, so there is nothing to update
    let Some(path) = path else {
        return Ok(());
    };
    let text = serde_json::to_string(map).map_err(|err| err.to_string())?;
    write_atomic(&path, &text).map_err(|err| format!("could not save {}: {err}", path.display()))
}

pub(super) fn claude_accounts(accounts: &[AccountConfig]) -> impl Iterator<Item = &AccountConfig> {
    accounts.iter().filter(|account| account.kind == "claude")
}

/// The Claude account in the default sign-in: the one `activate` put
/// there, else the account that uses `~/.claude` itself.
pub(super) fn active_claude(accounts: &[AccountConfig]) -> Option<String> {
    let chosen = active_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| find(accounts, id).is_some());
    chosen.or_else(|| {
        claude_accounts(accounts)
            .find(|account| account.dir.is_none())
            .map(|account| account.id.clone())
    })
}

/// The slots orcha moved accounts into, dropping unknown accounts.
pub(super) fn read_slots(accounts: &[AccountConfig]) -> HashMap<String, String> {
    let mut slots = read_string_map(slots_file());
    slots.retain(|_, id| find(accounts, id).is_some());
    slots
}

/// The account whose sign-in is in a folder's slot now.
pub(super) fn slot_owner(accounts: &[AccountConfig], service: &str) -> Option<String> {
    read_slots(accounts).get(service).cloned().or_else(|| {
        claude_accounts(accounts)
            .find(|account| crate::platform::orcha_claude_service(&folder_slot(account)) == service)
            .map(|account| account.id.clone())
    })
}

/// A slot named by its service: the folder behind it when known.
pub(super) fn service_slot(service: &str) -> OrchaSecretSlot {
    let dirs = read_string_map(slot_dirs_file());
    match dirs.get(service) {
        Some(dir) => OrchaSecretSlot::Folder(Some(dir.clone())),
        None => OrchaSecretSlot::Service(service.to_owned()),
    }
}

/// When a sign-in's access token runs out (ms since the epoch), 0 if unknown.
pub(super) fn expires_at(secret: &str) -> f64 {
    serde_json::from_str::<Value>(secret)
        .ok()
        .and_then(|value| value.get("claudeAiOauth")?.get("expiresAt")?.as_f64())
        .unwrap_or(0.0)
}

/// Where an account's sign-in is now: the default folder when it is the
/// active account; else, of every place it may sit (folders it was switched
/// into, the copy orcha kept, its own folder), the one whose access token
/// runs out last. Old slot records can point at a stale copy, so the newest
/// wins over the record. The default folder belongs to the active account
/// only, so an inactive account never reads it.
pub(super) fn claude_slot(accounts: &[AccountConfig], id: &str) -> Option<OrchaSecretSlot> {
    let account = find(accounts, id)?;
    if active_claude(accounts).as_deref() == Some(id) {
        return Some(OrchaSecretSlot::Folder(None));
    }
    let mut candidates: Vec<OrchaSecretSlot> = read_slots(accounts)
        .into_iter()
        .filter(|(_, owner)| owner == id)
        .map(|(service, _)| service_slot(&service))
        .collect();
    candidates.push(kept_slot(id));
    if account.dir.is_some() {
        candidates.push(folder_slot(account));
    }
    candidates
        .into_iter()
        .filter_map(|slot| {
            let secret = secret_cache::read(&slot)?;
            Some((expires_at(&secret), slot))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, slot)| slot)
        // nothing anywhere yet: where a sign-in would be kept
        .or_else(|| Some(kept_slot(id)))
}

/// Before `id` signs in where Claude will write its sign-in: an account of
/// the default folder becomes the active one first (the sign-in there now,
/// another account's, is kept for it); an account whose folder another
/// account was switched into gets the folder back the same way.
pub(super) fn prepare_sign_in(accounts: &[AccountConfig], id: &str) -> Result<(), String> {
    let account = find(accounts, id).ok_or("unknown account")?;
    if account.kind != "claude" {
        return Ok(());
    }
    let slot = folder_slot(account);
    if account.dir.is_none() {
        let current = active_claude(accounts);
        if current.as_deref() == Some(id) {
            return Ok(());
        }
        if let Some(current) = &current {
            keep_for(current, &slot)?;
        }
        return save_active(id);
    }
    let service = crate::platform::orcha_claude_service(&slot);
    let before = read_slots(accounts);
    if let Some(owner) = before.get(&service).cloned().filter(|owner| owner != id) {
        // forget the owner first (Claude is about to write `id`'s sign-in
        // here), then keep its sign-in; put the record back if that fails
        let mut slots = before.clone();
        slots.remove(&service);
        write_string_map(slots_file(), &slots)?;
        if let Err(err) = keep_for(&owner, &slot) {
            if let Err(undo) = write_string_map(slots_file(), &before) {
                tracing::warn!(error = %undo, "orcha: could not restore the Claude slot record");
            }
            return Err(err);
        }
    }
    Ok(())
}

/// Records `id` as the account in the default sign-in (claude-active).
pub(super) fn save_active(id: &str) -> Result<(), String> {
    let path = active_file().ok_or("no config folder")?;
    write_atomic(&path, &format!("{id}\n"))
        .map_err(|err| format!("could not save the active account: {err}"))
}

/// Forgets which account the default sign-in belongs to (none had it).
pub(super) fn clear_active() -> Result<(), String> {
    let Some(path) = active_file() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("could not reset the active account: {err}"))
        }
        _ => Ok(()),
    }
}

/// Keeps the default folder's profile (`oauthAccount`) for `owner`.
pub(super) fn keep_profile(owner: &str, claude_json: &Path) -> Result<(), String> {
    let profile = read_json(claude_json).and_then(|state| state.get("oauthAccount").cloned());
    let (Some(profile), Some(dir)) = (profile, profile_dir()) else {
        return Ok(());
    };
    write_atomic(&dir.join(format!("{owner}.json")), &profile.to_string())
        .map_err(|err| format!("could not keep {owner}'s profile: {err}"))
}

/// Keeps the sign-in in `slot` (and, for the default folder, its profile)
/// for `owner`, so that account still works after `slot` changes hands.
pub(super) fn keep_for(owner: &str, slot: &OrchaSecretSlot) -> Result<(), String> {
    if let Some(live) = crate::platform::orcha_secret_read(slot) {
        secret_cache::write(&kept_slot(owner), &live, slot)
            .map_err(|err| format!("could not keep {owner}'s sign-in: {err}"))?;
    }
    if *slot == OrchaSecretSlot::Folder(None) {
        if let Some(home) = home() {
            keep_profile(owner, &home.join(".claude.json"))?;
        }
    }
    forget_limits(owner);
    Ok(())
}

/// An account's profile (`oauthAccount`: email etc.) as last seen.
pub(super) fn claude_profile(accounts: &[AccountConfig], id: &str) -> Option<Value> {
    if let Some(profile) = profile_dir().and_then(|dir| read_json(&dir.join(format!("{id}.json"))))
    {
        return Some(profile);
    }
    let dir = find(accounts, id)?.dir.clone()?;
    read_json(&Path::new(&dir).join(".claude.json"))?
        .get("oauthAccount")
        .cloned()
}
