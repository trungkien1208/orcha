/// Where each Claude account's sign-in is read from and when its access
/// token runs out (no tokens printed).
#[test]
#[ignore]
fn print_sign_ins() {
    let accounts = super::accounts();
    let now = super::now_secs();
    for account in accounts.iter().filter(|account| account.kind == "claude") {
        let slot = super::claude_slot(&accounts, &account.id);
        let secret = slot.as_ref().and_then(crate::platform::orcha_secret_read);
        let refresh = secret.as_deref().is_some_and(|secret| {
            serde_json::from_str::<serde_json::Value>(secret)
                .ok()
                .and_then(|value| value.get("claudeAiOauth")?.get("refreshToken").cloned())
                .is_some()
        });
        let expires = secret.as_deref().map(super::expires_at).unwrap_or(0.0) / 1000.0;
        eprintln!(
            "signin {} slot={:?} found={} refresh_token={} expires_in_min={:.0}",
            account.id,
            slot.map(|slot| crate::platform::orcha_claude_service(&slot)),
            secret.is_some(),
            refresh,
            (expires - now) / 60.0
        );
    }
}

/// Fetches every account once, past the cache (no tokens printed).
#[test]
#[ignore]
fn fetch_fresh() {
    let accounts = super::accounts();
    for account in &accounts {
        let limits = super::limits(&accounts, account, true);
        eprintln!(
            "fresh {} error={:?} session={:?} weekly={:?} monthly={:?}",
            account.id, limits.error, limits.session, limits.weekly, limits.monthly
        );
    }
}

/// `cargo test --bin herdr orcha_accounts::live -- --ignored --nocapture`
/// prints this machine's accounts and fuel (no tokens).
#[test]
#[ignore]
fn print_accounts() {
    let pane = std::env::var("ORCHA_LIVE_PANE").ok();
    for account in super::accounts_with_fuel(pane.as_deref(), false) {
        eprintln!(
            "{} {} email={:?} signed_in={:?} current={} error={:?} windows={:?}",
            account.id,
            account.kind,
            account.email,
            account.signed_in,
            account.current,
            account.error,
            account
                .windows
                .iter()
                .map(|window| format!("{} {}% {:?}", window.label, window.left, window.reset))
                .collect::<Vec<_>>()
        );
    }
}
