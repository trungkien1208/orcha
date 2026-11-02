//! orcha: user flows around popups, notices and agent prompts.
use super::orcha_layout::orcha_state;
use super::*;
use crate::client::shell::accounts::{Account, FuelWindow};
use crate::client::shell::orcha_popup::OrchaPopup;

fn account(id: &str, kind: &str, current: bool, left: u8) -> Account {
    Account {
        id: id.into(),
        kind: kind.into(),
        label: format!("Claude · {id}"),
        email: None,
        current,
        error: None,
        windows: vec![FuelWindow {
            key: "5h".into(),
            label: "5H".into(),
            left,
            reset: None,
            reset_at: None,
        }],
        signed_in: Some(true),
    }
}

fn set_focused_status(state: &mut ClientShellState, status: AgentStatus) {
    let snapshot = state.snapshot.as_deref_mut().expect("snapshot");
    for agent in &mut snapshot.agents {
        if agent.pane_id == "pane_1" {
            agent.agent_status = status;
        }
    }
}

/// The focused pane runs a plain shell: no agent row for it.
fn focus_shell(state: &mut ClientShellState) {
    let snapshot = state.snapshot.as_deref_mut().expect("snapshot");
    snapshot.agents.retain(|agent| agent.pane_id != "pane_1");
}

fn notice(state: &ClientShellState) -> String {
    state
        .copy_feedback
        .as_ref()
        .map(|feedback| feedback.message.clone())
        .unwrap_or_default()
}

fn key(code: KeyCode) -> crate::input::TerminalKey {
    crate::input::TerminalKey::new(code, KeyModifiers::NONE)
}

fn click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.handle_mouse(
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    outcome
}

#[test]
fn the_low_fuel_offer_is_a_notice_and_the_keys_stay_with_the_pane() {
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 8),
        account("personal", "claude", false, 70),
    ]));
    set_focused_status(&mut state, AgentStatus::Idle);

    assert!(state.offer_continue_on_low_fuel());
    assert!(state.orcha_ui.popup.is_none(), "nothing grabs the keyboard");
    assert!(
        notice(&state).contains("8% fuel left") && notice(&state).contains("personal"),
        "{}",
        notice(&state)
    );
    assert!(
        !state.offer_continue_on_low_fuel(),
        "offered once per account"
    );

    // typing a prompt and pressing enter goes to the pane, not to a popup
    state.handle_input_bytes(b"fix it\r");
    assert!(state.orcha_ui.popup.is_none());
    assert!(notice(&state).contains("8% fuel left"), "the offer stays");

    // the offer stays past the usual fade, until it is clicked
    assert!(
        !state.tick_copy_feedback(std::time::Instant::now() + std::time::Duration::from_secs(60))
    );
    state.compose(140, 40).expect("frame");
    let offer = state.hits.notice;
    assert!(!offer.is_empty());
    click(&mut state, offer);
    assert!(matches!(
        state.orcha_ui.popup,
        Some(OrchaPopup::Accounts { selected: 1, .. })
    ));
    assert!(state.copy_feedback.is_none(), "the click used the offer up");
}

#[test]
fn commit_and_pr_prompts_go_only_to_an_agent_waiting_for_one() {
    const PROMPT: &str = "Commit the changes.";
    let mut busy = orcha_state(true, true);
    busy.ask_focused_agent(PROMPT);
    assert_eq!(
        notice(&busy),
        "The agent is busy. Try again when it is done"
    );
    assert!(busy.orcha_ui.notice_jobs.is_empty());

    let mut asking = orcha_state(true, true);
    set_focused_status(&mut asking, AgentStatus::Blocked);
    asking.ask_focused_agent(PROMPT);
    assert!(
        asking.orcha_ui.notice_jobs.is_empty(),
        "an agent asking you something would take the enter as its answer"
    );

    let mut shell = orcha_state(true, true);
    focus_shell(&mut shell);
    shell.ask_focused_agent(PROMPT);
    assert_eq!(notice(&shell), "Open an agent first");
    assert!(shell.orcha_ui.notice_jobs.is_empty());

    // an idle agent gets it (the thread is refused here, so nothing is typed)
    let mut idle = orcha_state(true, true);
    set_focused_status(&mut idle, AgentStatus::Idle);
    crate::thread_spawn::test_hook::fail_next_spawns(1);
    idle.ask_focused_agent(PROMPT);
    assert_eq!(notice(&idle), "Could not start that, try again");
}

#[test]
fn auto_handoff_does_not_claim_success_when_the_account_job_cannot_start() {
    let mut state = orcha_state(true, true);
    state.config.orcha.checklist_percent = 0;
    state.config.orcha.auto_handoff_percent = 5;
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 3),
        account("personal", "claude", false, 70),
    ]));
    set_focused_status(&mut state, AgentStatus::Idle);
    crate::thread_spawn::test_hook::fail_next_spawns(1);

    assert!(state.act_on_low_fuel());
    assert!(state.orcha_ui.account_run.is_none());
    assert_eq!(
        state.accounts.current().expect("current account").id,
        "work"
    );
    assert!(!state.orcha_ui.auto_handed_off.contains("work"));
    assert_eq!(
        notice(&state),
        "Could not start the account change, try again"
    );
}

#[test]
fn hand_off_without_an_agent_says_so() {
    let mut state = orcha_state(true, true);
    focus_shell(&mut state);
    state.open_handoff_popup();
    assert!(state.orcha_ui.popup.is_none());
    assert_eq!(notice(&state), "hand off: open an agent first");
}

#[test]
fn new_agent_starts_on_the_default_account_unless_it_is_low() {
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 90),
        account("personal", "claude", false, 40),
    ]));
    state.orcha_ui.default_accounts = vec!["personal".into()];
    state.open_new_agent_popup();
    assert!(
        matches!(
            state.orcha_ui.popup,
            Some(OrchaPopup::NewAgent { selected: 1, .. })
        ),
        "the default, not the one with most fuel"
    );

    state.orcha_ui.popup = None;
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 90),
        account("personal", "claude", false, 3),
    ]));
    state.open_new_agent_popup();
    assert!(
        matches!(
            state.orcha_ui.popup,
            Some(OrchaPopup::NewAgent { selected: 0, .. })
        ),
        "a default running dry gives way to the most fuel"
    );
}

#[test]
fn esc_in_an_account_sub_popup_goes_back_to_the_accounts() {
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 60),
        account("personal", "claude", false, 70),
    ]));
    let mut outcome = ClientShellInput::default();
    state.orcha_ui.popup = Some(OrchaPopup::Accounts {
        selected: 1,
        pane: Some("pane_1".into()),
    });
    state.route_orcha_popup_key(&key(KeyCode::Char('d')), &mut outcome);
    assert!(matches!(
        state.orcha_ui.popup,
        Some(OrchaPopup::RemoveAccount { .. })
    ));
    state.route_orcha_popup_key(&key(KeyCode::Esc), &mut outcome);
    assert!(
        matches!(
            state.orcha_ui.popup,
            Some(OrchaPopup::Accounts { selected: 1, .. })
        ),
        "back on the account it was about"
    );

    state.orcha_ui.popup = Some(OrchaPopup::SwitchAccount {
        selected: 0,
        id: "personal".into(),
        label: "Claude · personal".into(),
        pane: "pane_1".into(),
        restart_blocked: None,
    });
    state.route_orcha_popup_key(&key(KeyCode::Esc), &mut outcome);
    assert!(matches!(
        state.orcha_ui.popup,
        Some(OrchaPopup::Accounts { selected: 1, .. })
    ));

    let handoff = OrchaPopup::Handoff {
        selected: 0,
        pane: "pane_1".into(),
        targets: vec![0],
    };
    state.orcha_ui.popup = Some(handoff.clone());
    state.orcha_ui.handoff_from_accounts = true;
    state.route_orcha_popup_key(&key(KeyCode::Esc), &mut outcome);
    assert!(matches!(
        state.orcha_ui.popup,
        Some(OrchaPopup::Accounts { .. })
    ));

    // opened from COMMANDS: nothing to go back to
    state.orcha_ui.popup = Some(handoff);
    state.orcha_ui.handoff_from_accounts = false;
    state.route_orcha_popup_key(&key(KeyCode::Esc), &mut outcome);
    assert!(state.orcha_ui.popup.is_none());
}

#[test]
fn a_working_notice_outlasts_other_notices_and_a_failure_stays_until_clicked() {
    let mut state = orcha_state(true, true);
    let later = |seconds| std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    state.start_working_notice("orcha-handoff", "hand off: writing the notes…".into());
    state.show_notice("refreshing fuel…".into());
    assert!(state.tick_copy_feedback(later(4)));
    assert_eq!(notice(&state), "hand off: writing the notes…");
    assert!(
        !state.tick_copy_feedback(later(60)),
        "it stays while the job runs"
    );
    state.end_working_notice("orcha-handoff");
    assert!(state.copy_feedback.is_none());

    state.show_failure("hand off failed: the pane is gone".into());
    assert!(!state.tick_copy_feedback(later(600)));
    assert_eq!(notice(&state), "hand off failed: the pane is gone");
    state.compose(140, 40).expect("frame");
    let failure = state.hits.notice;
    click(&mut state, failure);
    assert!(state.copy_feedback.is_none(), "a click puts it away");
}

#[test]
fn project_expanded_follows_selection_and_explicit_choices() {
    let mut ui = OrchaUiState::default();
    assert!(!ui.project_expanded("ws_2", false));
    assert!(
        ui.project_expanded("ws_2", true),
        "the selected project shows its agents"
    );
    ui.set_project_expanded("ws_2", false);
    assert!(
        !ui.project_expanded("ws_2", true),
        "folded on purpose stays folded"
    );
    ui.set_project_expanded("ws_2", true);
    assert!(
        ui.project_expanded("ws_2", false),
        "opened by hand stays open"
    );
    assert!(!ui.folded_projects.contains("ws_2"));
}

#[test]
fn orcha_update_news_lights_the_menu_and_adds_items() {
    let mut state = orcha_state(true, true);
    assert_eq!(state.orcha_ui.menu_news(), Default::default());
    state.orcha_ui.update_available = Some("9.9.9".into());
    let news = state.orcha_ui.menu_news();
    assert!(news.update && !news.plugin_update);
    let snapshot = state.snapshot.as_deref().expect("snapshot");
    let items = crate::client::shell::global_menu::global_menu_items(snapshot, news);
    let update = items
        .iter()
        .find(|(label, _)| *label == "update orcha")
        .map(|(_, action)| *action)
        .expect("an update item");
    assert!(crate::client::shell::global_menu::global_menu_item_has_badge(snapshot, update));
    assert!(!items.iter().any(|(label, _)| *label == "update plugins"));
}

#[test]
fn manual_switch_does_not_move_the_mark_when_the_job_cannot_start() {
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![
        account("work", "claude", true, 8),
        account("personal", "claude", false, 70),
    ]));
    state.orcha_ui.popup = Some(OrchaPopup::SwitchAccount {
        selected: 0,
        id: "personal".into(),
        label: "Personal".into(),
        pane: "pane_1".into(),
        restart_blocked: None,
    });
    state.compose(140, 40).expect("switch popup");
    crate::thread_spawn::test_hook::fail_next_spawns(1);
    state.route_orcha_popup_key(&key(KeyCode::Enter), &mut ClientShellInput::default());
    assert!(state.orcha_ui.account_run.is_none());
    assert_eq!(state.accounts.current().expect("current").id, "work");
    assert!(notice(&state).contains("Could not start"));
}

#[test]
fn a_fast_focus_change_uses_the_popups_agent_for_the_handoff() {
    use crate::client::shell::orcha_popup::PopupTarget;
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![
        account("previous-pane", "codex", true, 80),
        account("this-pane", "claude", false, 60),
    ]));
    // The account poller has not followed a quick focus change to Claude yet.
    state.orcha_ui.popup = Some(OrchaPopup::Accounts {
        selected: 0,
        pane: Some("pane_1".into()),
    });
    state.compose(140, 40).expect("accounts");
    assert!(
        state
            .hits
            .orcha
            .popup
            .actions
            .iter()
            .any(|(_, target)| { *target == PopupTarget::Row(0) }),
        "Codex offers a handoff action instead of being marked in use by Claude"
    );
    // Exercise routing without starting a real handoff or touching credentials.
    crate::thread_spawn::test_hook::fail_next_spawns(1);
    state.route_orcha_popup_key(&key(KeyCode::Enter), &mut ClientShellInput::default());
    assert!(state.orcha_ui.popup.is_none(), "the handoff was dispatched");
    assert!(state.orcha_ui.handoff_run.is_none());
    assert_eq!(
        state.accounts.current().expect("current").id,
        "previous-pane"
    );
}

#[test]
fn account_marks_follow_success_for_the_same_local_pane_and_provider() {
    use crate::client::shell::background_job::BackgroundJob;
    for (success, same_provider, local, expected) in [
        (false, true, true, "work"),
        (true, false, true, "work"),
        (true, true, false, "work"),
        (true, true, true, "personal"),
    ] {
        let mut state = orcha_state(true, true);
        state.accounts.set_for_test(Some(vec![
            account("work", "claude", true, 8),
            account("personal", "claude", false, 70),
        ]));
        state.orcha_ui.account_change = Some(("personal".into(), "pane_1".into(), "claude".into()));
        state.orcha_ui.account_run = BackgroundJob::spawn("test-account-result", move || {
            if success {
                Ok("changed".into())
            } else {
                Err("rejected".into())
            }
        });
        if !same_provider {
            state
                .snapshot
                .as_deref_mut()
                .expect("snapshot")
                .agents
                .iter_mut()
                .find(|agent| agent.pane_id == "pane_1")
                .expect("agent")
                .agent = Some("codex".into());
        }
        if !local {
            state.active_endpoint_id = crate::client::shell::ClientEndpointId::Ssh(
                crate::client::endpoint::ProfileId::generate(),
            );
        }
        let mut finished = false;
        for _ in 0..2_000 {
            if state.poll_account_action() {
                finished = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(finished);
        assert_eq!(state.accounts.current().expect("current").id, expected);
        assert!(state.orcha_ui.account_change.is_none());
    }
}
