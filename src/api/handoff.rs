//! `agent.handoff`: moves a pane's conversation to a new agent, optionally on
//! another account, with notes built from the old agent's transcript. The old
//! agent is not asked for anything, so this works when it is out of fuel.

use crate::api::schema::{AgentHandoffParams, ResponseResult};
use crate::api::ApiRequestSender;

type HandoffError = (&'static str, String);

pub(super) fn handle(
    params: AgentHandoffParams,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, HandoffError> {
    let _ = api_tx;
    let mode = crate::orcha_handoff::Mode::from_notes(params.notes.as_deref().unwrap_or("full"))
        .ok_or((
            "invalid_params",
            "notes must be full or summary".to_string(),
        ))?;
    let (kind, claude_dir) = target(params.to.kind.as_deref(), params.to.account_id.as_deref())?;
    let (_, pane_id) =
        crate::orcha_handoff::hand_off_to(&params.pane_id, &kind, mode, claude_dir.as_deref())
            .map_err(|message| ("handoff_failed", message))?;
    Ok(ResponseResult::AgentHandedOff { pane_id, kind })
}

/// Starts a new agent for `account_id` with a hand-off of `pane_id`'s
/// conversation, returning the new pane.
pub(super) fn handoff_to_account(
    _request_id: &str,
    pane_id: &str,
    account_id: &str,
    _api_tx: &ApiRequestSender,
) -> Result<String, HandoffError> {
    let (kind, claude_dir) = target(None, Some(account_id))?;
    crate::orcha_handoff::hand_off_to(
        pane_id,
        &kind,
        crate::orcha_handoff::Mode::Full,
        claude_dir.as_deref(),
    )
    .map(|(_, pane)| pane)
    .map_err(|message| ("handoff_failed", message))
}

/// The agent kind to start and, for a Claude account, its sign-in folder.
fn target(
    kind: Option<&str>,
    account_id: Option<&str>,
) -> Result<(String, Option<String>), HandoffError> {
    let Some(account_id) = account_id else {
        let kind = kind.ok_or((
            "invalid_params",
            "to needs a kind or an account_id".to_string(),
        ))?;
        if !crate::orcha_handoff::AGENTS
            .iter()
            .any(|(known, _, _)| *known == kind)
        {
            return Err(("unsupported_agent", format!("cannot hand off to {kind}")));
        }
        return Ok((kind.to_string(), None));
    };
    let accounts = crate::orcha_accounts::accounts();
    let account = crate::orcha_accounts::find_account(&accounts, account_id)
        .ok_or(("account_not_found", format!("no account {account_id}")))?;
    if kind.is_some_and(|kind| kind != account.kind) {
        return Err((
            "invalid_params",
            format!("{account_id} is a {} account", account.kind),
        ));
    }
    crate::orcha_accounts::prepare_folder(account);
    Ok((account.kind.clone(), account.dir.clone()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn handoff_targets_need_a_known_kind_or_account() {
        assert_eq!(super::target(None, None).unwrap_err().0, "invalid_params");
        assert_eq!(
            super::target(Some("vim"), None).unwrap_err().0,
            "unsupported_agent"
        );
        assert_eq!(
            super::target(Some("codex"), None).unwrap(),
            ("codex".to_string(), None)
        );
    }
}
