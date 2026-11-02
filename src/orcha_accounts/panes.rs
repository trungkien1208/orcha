use super::*;

// ---------- panes

/// A pane's `herdr pane get` info.
pub(super) fn pane_request(method: crate::api::schema::Method) -> Option<Value> {
    let request = crate::api::schema::Request {
        id: "orcha:account-pane".into(),
        method,
    };
    let client = crate::api::client::ApiClient::for_target(
        crate::api::client::ConnectionTarget::SocketPath(crate::session::active_api_socket_path()),
    );
    let mut reply = client
        .request_value_with_timeout(&request, std::time::Duration::from_secs(2))
        .ok()?;
    reply.get_mut("result").map(Value::take)
}

pub(super) fn pane_info(pane: &str) -> Option<Value> {
    let mut result = pane_request(crate::api::schema::Method::PaneGet(
        crate::api::schema::PaneTarget {
            pane_id: pane.to_owned(),
        },
    ))?;
    result.get_mut("pane").map(Value::take)
}

/// `CLAUDE_CONFIG_DIR` of the Claude running in a pane, as Claude sees it;
/// None for the default `~/.claude` (or when the host cannot tell).
pub(super) fn pane_config_dir(pane: &str) -> Option<String> {
    let info = pane_request(crate::api::schema::Method::PaneProcessInfo(
        crate::api::schema::PaneProcessInfoParams {
            pane_id: Some(pane.to_owned()),
        },
    ))?;
    info.get("process_info")?
        .get("foreground_processes")?
        .as_array()?
        .iter()
        .filter_map(|process| process.get("pid")?.as_u64())
        .find_map(|pid| crate::platform::orcha_process_env_var(pid as u32, "CLAUDE_CONFIG_DIR"))
        .filter(|dir| !dir.is_empty())
}

/// The account the agent in `pane` runs on.
pub(super) fn pane_account(accounts: &[AccountConfig], pane: &str) -> Option<String> {
    let info = pane_info(pane)?;
    let kind = info.get("agent").and_then(Value::as_str)?;
    agent_account(accounts, kind, pane)
}

pub(super) fn agent_account(accounts: &[AccountConfig], kind: &str, pane: &str) -> Option<String> {
    if kind != "claude" {
        return accounts
            .iter()
            .find(|account| account.kind == kind)
            .map(|account| account.id.clone());
    }
    let Some(dir) = pane_config_dir(pane) else {
        return active_claude(accounts);
    };
    let service =
        crate::platform::orcha_claude_service(&OrchaSecretSlot::Folder(Some(dir.clone())));
    if let Some(owner) = slot_owner(accounts, &service) {
        return Some(owner);
    }
    let found = std::fs::canonicalize(expand(&dir)).ok();
    claude_accounts(accounts)
        .find(|account| {
            account
                .dir
                .as_deref()
                .and_then(|own| std::fs::canonicalize(own).ok())
                .is_some_and(|own| Some(own) == found)
        })
        .map(|account| account.id.clone())
}

/// Every account and its fuel; `current` is `pane`'s account, else the
/// one picked last.
pub(crate) fn accounts_with_fuel(pane: Option<&str>, fresh: bool) -> Vec<Account> {
    accounts_with_fuel_of(&accounts(), pane, fresh)
}

/// `accounts_with_fuel` for accounts the caller has already read.
pub(crate) fn accounts_with_fuel_of(
    accounts: &[AccountConfig],
    pane: Option<&str>,
    fresh: bool,
) -> Vec<Account> {
    if fresh {
        secret_cache::invalidate_all();
    }
    let current = pane
        .and_then(|pane| pane_account(accounts, pane))
        .or_else(|| selected(accounts));
    let current = current.as_deref();
    let now = now_secs();
    let one = |account: &AccountConfig| {
        let fuel = agent_kind(&account.kind).is_some_and(|agent| agent.fuel);
        let limits = if fuel {
            limits(accounts, account, fresh)
        } else {
            Limits::default()
        };
        Account {
            id: account.id.clone(),
            kind: account.kind.clone(),
            label: account.label.clone(),
            email: email(accounts, account),
            current: current == Some(account.id.as_str()),
            windows: windows(&limits, now),
            signed_in: Some(signed_in(accounts, account) && !auth_needed(limits.error.as_deref())),
            error: limits.error,
        }
    };
    // each request can take up to 15 s: ask every account at once
    std::thread::scope(|scope| {
        let asked: Vec<_> = accounts
            .iter()
            .map(|account| {
                let thread = std::thread::Builder::new()
                    .name("orcha-fuel".to_owned())
                    .spawn_scoped(scope, move || one(account));
                (account, thread)
            })
            .collect();
        asked
            .into_iter()
            .map(|(account, thread)| match thread {
                Ok(thread) => thread.join().unwrap_or_else(|_| Account {
                    id: account.id.clone(),
                    kind: account.kind.clone(),
                    label: account.label.clone(),
                    email: None,
                    current: current == Some(account.id.as_str()),
                    windows: Vec::new(),
                    signed_in: None,
                    error: Some("could not read its fuel".to_owned()),
                }),
                // no thread to spare: ask here instead
                Err(_) => one(account),
            })
            .collect()
    })
}
