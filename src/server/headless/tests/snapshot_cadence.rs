use super::*;

fn report_state(
    server: &mut HeadlessServer,
    pane_id: crate::layout::PaneId,
    state: crate::detect::AgentState,
) {
    server.app.state.ensure_test_terminals();
    let updates = server.app.state.handle_app_event(AppEvent::StateChanged {
        pane_id,
        agent: Some(crate::detect::Agent::Pi),
        state,
        visible_blocker: false,
        visible_working: state == crate::detect::AgentState::Working,
        process_exited: false,
        observed_at: Instant::now(),
    });
    assert!(
        !updates.is_empty(),
        "the fixture must observe the state report"
    );
}

#[tokio::test]
async fn passive_projection_flushes_its_final_state_without_more_output_even_for_inactive_surfaces()
{
    let (mut server, control, render, pane_id) = retained_test_server_with_control(b"baseline");
    server.clients.get_mut(&1).unwrap().shell_surface_active = false;
    report_state(&mut server, pane_id, crate::detect::AgentState::Working);
    let now = Instant::now();
    server.render_and_stream_coalesced(now);
    let (before, _) = client_shell_projection(&control);
    report_state(&mut server, pane_id, crate::detect::AgentState::Idle);
    server.render_and_stream_coalesced(now + Duration::from_millis(10));
    assert!(control.try_recv().is_err());
    assert!(render.try_recv().is_err());
    assert_eq!(
        server.clients[&1].shell_projection_revision,
        before.revision
    );
    assert_eq!(
        server.shell_snapshot_deadline(),
        Some(now + Duration::from_millis(100))
    );
    assert!(!server.shell_snapshot_due(now + Duration::from_millis(99)));
    assert!(server.shell_snapshot_due(now + Duration::from_millis(100)));
    server.render_and_stream_coalesced(now + Duration::from_millis(100));
    let (after, completions) = client_shell_projection(&control);
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.agents[0].state_change_seq > before.agents[0].state_change_seq);
    assert_eq!(after.revision, completions.revision);
    assert!(server.shell_snapshot_deadline().is_none());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn pane_pixels_continue_on_the_prior_coherent_projection_during_passive_batching() {
    let (mut server, control, render, pane_id) = retained_test_server_with_control(b"baseline");
    report_state(&mut server, pane_id, crate::detect::AgentState::Working);
    let now = Instant::now();
    server.render_and_stream_coalesced(now);
    let (before, _) = client_shell_projection(&control);
    let initial = recv_pane_surface(&render, "initial surface");
    assert_eq!(initial.projection_revision, before.revision);
    report_state(&mut server, pane_id, crate::detect::AgentState::Idle);
    write_shared_test_pane(&mut server, pane_id, b"\rupdated pixels");
    server.render_and_stream_coalesced(now + Duration::from_millis(10));
    assert!(control.try_recv().is_err(), "metadata waits");
    let updated = recv_pane_surface(&render, "responsive surface");
    assert_eq!(updated.projection_revision, before.revision);
    assert!(frame_text(&updated.frame).contains("updated pixels"));
    assert!(updated.surface_revision > initial.surface_revision);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn urgent_and_manual_renders_bypass_passive_delay_and_surface_activation_advances_the_floor()
{
    let (mut server, control, _render, pane_id) = retained_test_server_with_control(b"baseline");
    server.clients.get_mut(&1).unwrap().shell_surface_active = false;
    report_state(&mut server, pane_id, crate::detect::AgentState::Working);
    let now = Instant::now();
    server.render_and_stream_coalesced(now);
    let before = client_shell_snapshot(&control);
    report_state(&mut server, pane_id, crate::detect::AgentState::Idle);
    server.render_and_stream_coalesced(now + Duration::from_millis(10));
    server.invalidate_shell_snapshot_cadence();
    server.render_and_stream_coalesced(now + Duration::from_millis(11));
    let after = client_shell_snapshot(&control);
    assert!(after.revision > before.revision);
    report_state(&mut server, pane_id, crate::detect::AgentState::Working);
    server.render_and_stream_coalesced(now + Duration::from_millis(12));
    assert!(control.try_recv().is_err());
    server.render_and_stream();
    let manual = client_shell_snapshot(&control);
    assert!(manual.revision > after.revision);
    let (_, floor) = server
        .set_client_shell_surface_active(1, true)
        .expect("active lease");
    assert!(floor > manual.revision);
    assert!(server.clients[&1].shell_snapshot.is_none());
    assert!(server.clients[&1]
        .shell_snapshot_cadence
        .deadline()
        .is_none());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn synchronized_surface_retries_do_not_leave_an_expired_snapshot_deadline() {
    let (mut server, control, render, pane_id) = retained_test_server_with_control(b"baseline");
    report_state(&mut server, pane_id, crate::detect::AgentState::Working);
    let now = Instant::now();
    server.render_and_stream_coalesced(now);
    let _ = client_shell_snapshot(&control);
    let _ = recv_pane_surface(&render, "baseline");
    report_state(&mut server, pane_id, crate::detect::AgentState::Idle);
    server.render_and_stream_coalesced(now + Duration::from_millis(1));
    write_shared_test_pane(&mut server, pane_id, b"\x1b[?2026hpartial");
    server.render_and_stream_coalesced(now + Duration::from_millis(100));
    assert!(control.try_recv().is_err());
    assert!(!server.shell_snapshot_due(now + Duration::from_millis(100)));
    assert_eq!(
        server.shell_snapshot_deadline(),
        Some(now + Duration::from_millis(200))
    );
    shutdown_test_runtimes(&mut server);
}
