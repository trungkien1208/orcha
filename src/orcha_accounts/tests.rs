use super::*;

fn signed_in_folder(root: &Path, name: &str, email: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create folder");
    std::fs::write(
        dir.join(".claude.json"),
        serde_json::json!({"oauthAccount": {"emailAddress": email}}).to_string(),
    )
    .expect("write sign-in");
    dir
}

#[test]
fn found_accounts_skip_listed_dismissed_and_signed_out_folders() {
    let root = std::env::temp_dir().join(format!("orcha-found-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let listed = signed_in_folder(&root, ".claude-listed", "a@x.io");
    let work = signed_in_folder(&root, ".claude-work", "w@x.io");
    let gone = signed_in_folder(&root, ".claude-gone", "g@x.io");
    let empty = root.join(".claude-empty");
    std::fs::create_dir_all(&empty).expect("create empty");
    let config = serde_json::json!({
        "accounts": [{"id": "claude-a", "kind": "claude", "dir": listed.to_string_lossy()}],
        "dismissed_account_dirs": [gone.to_string_lossy()],
    });
    let candidates = [listed, work.clone(), gone.clone(), empty];
    let found = find_unlisted_in(
        Some(&config),
        &candidates,
        Some("me@x.io".into()),
        &[],
        false,
    );
    assert_eq!(
        found,
        vec![
            FoundAccount {
                kind: "claude".into(),
                dir: None,
                email: "me@x.io".into(),
                name: "me".into(),
                signed_in: true,
            },
            FoundAccount {
                kind: "claude".into(),
                dir: Some(work.to_string_lossy().into_owned()),
                email: "w@x.io".into(),
                name: "work".into(),
                signed_in: true,
            },
        ]
    );
    let all = find_unlisted_in(Some(&config), &candidates, None, &[], true);
    assert_eq!(all.len(), 2, "asking again brings dismissed ones back");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn importing_picks_free_ids_and_remembers_the_rest() {
    let mut config = serde_json::json!({"accounts": [
        {"id": "claude-work", "kind": "claude", "dir": null},
    ]});
    let claude = |dir: &str, name: &str| FoundAccount {
        kind: "claude".into(),
        dir: Some(dir.into()),
        email: format!("{name}@x.io"),
        name: name.into(),
        signed_in: true,
    };
    let gemini = FoundAccount {
        kind: "gemini".into(),
        dir: None,
        email: String::new(),
        name: "Gemini".into(),
        signed_in: false,
    };
    let added = import_into_config(
        &mut config,
        &[claude("/w", "work"), gemini],
        &[claude("/s", "side")],
    )
    .expect("import");
    assert_eq!(added, ["claude-work2", "gemini"]);
    let accounts = parse_accounts(&config);
    assert_eq!(accounts[1].dir.as_deref(), Some("/w"));
    assert_eq!(accounts[1].email, "work@x.io");
    assert_eq!(
        (accounts[2].kind.as_str(), accounts[2].label.as_str()),
        ("gemini", "Gemini")
    );
    assert_eq!(config["dismissed_account_dirs"], serde_json::json!(["/s"]));
}

#[test]
fn other_agents_are_offered_once_unless_listed_hidden_or_dismissed() {
    let config = serde_json::json!({
        "accounts": [{"id": "amp", "kind": "amp"}],
        "hidden_accounts": ["qwen"],
        "dismissed_account_dirs": ["agent:kimi"],
    });
    let agents = [
        ("gemini", Some("g@x.io".to_owned()), true),
        ("amp", None, true),
        ("qwen", None, true),
        ("kimi", None, false),
        ("codex", None, true),
        ("opencode", None, false),
    ];
    let found = find_unlisted_in(Some(&config), &[], None, &agents, false);
    let kinds: Vec<_> = found
        .iter()
        .map(|account| (account.kind.as_str(), account.signed_in))
        .collect();
    assert_eq!(kinds, [("gemini", true), ("opencode", false)]);
    assert_eq!(found[0].email, "g@x.io");
    assert_eq!(found[0].key(), "agent:gemini");
    let again = find_unlisted_in(Some(&config), &[], None, &agents, true);
    assert!(again.iter().any(|account| account.kind == "kimi"));
}

#[test]
fn every_agent_has_a_command_and_a_continue_line() {
    for agent in AGENTS {
        assert!(!agent.command.is_empty() && !agent.login.is_empty());
        assert!(continue_line(agent.kind).contains(agent.command));
    }
}

#[test]
fn config_accounts_keep_known_kinds_and_expand_folders() {
    let config = serde_json::json!({"accounts": [
        {"id": "claude-work", "kind": "claude", "label": "Claude · work", "email": "a@b.c", "dir": null},
        {"id": "claude-me", "kind": "claude", "label": "", "dir": "/x/me"},
        {"id": "odd", "kind": "not-an-agent"},
        {"kind": "codex"},
    ]});
    let accounts = parse_accounts(&config);
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].dir, None);
    assert_eq!(accounts[0].email, "a@b.c");
    assert_eq!(accounts[1].label, "claude-me");
    assert_eq!(accounts[1].dir.as_deref(), Some("/x/me"));
}

#[test]
fn iso_times_with_zones_and_fractions() {
    assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0.0));
    assert_eq!(iso_secs("2026-10-23T12:00:00Z"), Some(1_792_756_800.0));
    assert_eq!(
        iso_secs("2026-10-23T19:00:00+07:00"),
        iso_secs("2026-10-23T12:00:00Z")
    );
    assert_eq!(
        iso_secs("2026-10-23T12:00:00.500000Z"),
        Some(1_792_756_800.5)
    );
    assert_eq!(iso_secs("2026-10-23T12:00:00"), Some(1_792_756_800.0));
    assert_eq!(iso_secs("not a time"), None);
}

#[test]
fn reset_times_read_short() {
    assert_eq!(until(100.0, 200.0), "now");
    assert_eq!(until(200.0 + 12.0 * 60.0, 200.0), "12m");
    assert_eq!(until(3_900.0, 0.0), "1h 05m");
    assert_eq!(until(2.0 * 86_400.0 + 3.0 * 3600.0, 0.0), "2d 3h");
}

#[test]
fn windows_show_percent_left() {
    let limits = Limits {
        session: Some((30.4, Some(3_600.0))),
        weekly: Some((120.0, None)),
        ..Limits::default()
    };
    let shown = windows(&limits, 0.0);
    assert_eq!(shown.len(), 2);
    assert_eq!((shown[0].label.as_str(), shown[0].left), ("5H", 70));
    assert_eq!(shown[0].reset.as_deref(), Some("1h 00m"));
    assert_eq!((shown[1].label.as_str(), shown[1].left), ("WEEK", 0));
}

#[test]
fn windows_past_their_reset_read_full() {
    let limits = Limits {
        session: Some((80.0, Some(100.0))),
        weekly: Some((40.0, Some(9_000.0))),
        ..Limits::default()
    };
    let shown = windows(&limits, 200.0);
    assert_eq!((shown[0].left, shown[0].reset.as_deref()), (100, None));
    assert_eq!(shown[1].left, 60);
}

#[test]
fn cache_entries_read_the_python_format() {
    let python = r#"{"session": [42.0, 1792756800.0], "weekly": [10, null], "note": "x"}"#;
    let limits: Limits = serde_json::from_str(python).expect("python cache");
    assert_eq!(limits.session, Some((42.0, Some(1_792_756_800.0))));
    assert_eq!(limits.weekly, Some((10.0, None)));
    let back = serde_json::to_string(&limits).expect("json");
    assert!(back.contains("\"session\":[42.0,1792756800.0]"));
    assert!(!back.contains("monthly"));
}

#[test]
fn codex_logs_give_their_newest_limits_and_past_resets_count_as_empty() {
    let log = [
        r#"{"type":"event_msg","payload":{"rate_limits":{"primary":{"used_percent":10,"resets_at":100}}}}"#,
        r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
        r#"{"type":"event_msg","payload":{"rate_limits":{"primary":null}}}"#,
        r#"{"type":"event_msg","payload":{"rate_limits":{"primary":{"used_percent":55,"resets_at":5000},"secondary":{"used_percent":20,"resets_in_seconds":60}}}}"#,
    ]
    .join("\n");
    let limits = codex_rate_limits(&log).expect("limits");
    assert_eq!(
        codex_window(&limits, "primary", 1000.0),
        Some((55.0, Some(5000.0)))
    );
    assert_eq!(
        codex_window(&limits, "secondary", 1000.0),
        Some((20.0, Some(1060.0)))
    );
    assert_eq!(
        codex_window(&limits, "primary", 9000.0),
        Some((0.0, Some(5000.0)))
    );
}

#[test]
fn jwt_payloads_decode() {
    // {"sub":"auth0|user_1","email":"me@x.io"}
    let token = "e30.eyJzdWIiOiJhdXRoMHx1c2VyXzEiLCJlbWFpbCI6Im1lQHguaW8ifQ.sig";
    let payload = jwt_payload(token).expect("payload");
    assert_eq!(payload["sub"], "auth0|user_1");
    assert_eq!(payload["email"], "me@x.io");
}

#[test]
fn the_newer_sign_in_is_the_one_expiring_later() {
    let older = r#"{"claudeAiOauth":{"accessToken":"a","expiresAt":1000}}"#;
    let newer = r#"{"claudeAiOauth":{"accessToken":"b","expiresAt":2000}}"#;
    assert!(expires_at(newer) > expires_at(older));
    assert_eq!(expires_at("not json"), 0.0);
}

#[test]
fn sign_in_is_needed_by_kind_not_by_wording() {
    let refused = Fetched::needs_sign_in("whatever the words");
    assert!(refused.limits.needs_sign_in());
    // a message that mentions signing in is not enough
    let offline = Fetched::transient("request failed (sign in page down)".into(), false);
    assert!(!offline.limits.needs_sign_in());
    assert_eq!(offline.error_kind(), Some(FuelError::Transient));
    assert!(!Limits::default().needs_sign_in());
}

#[test]
fn the_error_kind_survives_the_cache_and_old_caches_still_read() {
    let refused = Fetched::needs_sign_in("no sign-in found: sign in").limits;
    let text = serde_json::to_string(&refused).expect("json");
    assert!(text.contains("\"error_kind\":\"needs_sign_in\""));
    let back: Limits = serde_json::from_str(&text).expect("read back");
    assert!(back.needs_sign_in());
    // limits.py and older orcha write no kind
    let old: Limits =
        serde_json::from_str(r#"{"error":"no sign-in found: sign in"}"#).expect("old cache");
    assert_eq!(old.error_kind, None);
    assert!(!old.needs_sign_in());
}

#[test]
fn adding_an_account_keeps_the_config_and_refuses_repeats() {
    let mut config =
        serde_json::json!({"lang": "en", "accounts": [{"id": "claude-work", "kind": "claude"}]});
    let id = add_to_config(&mut config, "Personal Mac").expect("added");
    assert_eq!(id, "claude-personal-mac");
    assert_eq!(config["lang"], "en");
    assert_eq!(
        config["accounts"][1]["dir"],
        "~/.ai-accounts/claude/personal-mac"
    );
    assert!(add_to_config(&mut config, "personal mac").is_err());
    assert!(add_to_config(&mut config, "work").is_err());
    assert!(add_to_config(&mut config, "  ").is_err());
}

#[test]
fn removing_drops_listed_accounts_and_hides_detected_ones() {
    let mut config =
        serde_json::json!({"lang": "en", "accounts": [{"id": "claude-work", "kind": "claude"}]});
    remove_from_config(&mut config, "claude-work").expect("removed");
    assert_eq!(config["accounts"], serde_json::json!([]));
    assert!(config.get("hidden_accounts").is_none());
    remove_from_config(&mut config, "codex").expect("hidden");
    remove_from_config(&mut config, "codex").expect("hidden once");
    assert_eq!(config["hidden_accounts"], serde_json::json!(["codex"]));
    assert_eq!(config["lang"], "en");
    assert_eq!(hidden_accounts(Some(&config)), vec!["codex".to_owned()]);
}

#[test]
fn slot_names_match_claude_and_the_python_script() {
    use crate::platform::orcha_claude_service;
    assert_eq!(
        orcha_claude_service(&OrchaSecretSlot::Folder(None)),
        "Claude Code-credentials"
    );
    // sha256("/Users/me/.ai-accounts/claude/personal")[:8]
    let name = orcha_claude_service(&OrchaSecretSlot::Folder(Some(
        "/Users/me/.ai-accounts/claude/personal".to_owned(),
    )));
    assert_eq!(name, "Claude Code-credentials-69098b02");
    assert_eq!(
        orcha_claude_service(&OrchaSecretSlot::Kept("claude-work".to_owned())),
        "orcha-claude-claude-work"
    );
}

#[test]
fn pane_ids_are_found_anywhere_in_a_result() {
    let result = serde_json::json!({"tab": {"panes": [{"pane_id": "w1:p2"}]}});
    assert_eq!(first_pane_id(&result).as_deref(), Some("w1:p2"));
    assert_eq!(first_pane_id(&serde_json::json!({"tab": {}})), None);
}

#[test]
fn only_plain_emails_go_on_the_command_line() {
    assert_eq!(plain_email(" me+x@ex.io "), Some("me+x@ex.io"));
    assert_eq!(plain_email("a@b.c; rm -rf ~"), None);
    assert_eq!(plain_email(""), None);
}

#[test]
fn only_a_refused_cursor_sign_in_asks_to_sign_in_again() {
    for status in [401, 403] {
        let refused = cursor_failed(&CurlError::from_stderr(&format!(
            "curl: (22) The requested URL returned error: {status}"
        )));
        assert_eq!(
            refused.error_kind(),
            Some(FuelError::NeedsSignIn),
            "{status}"
        );
    }
    for stderr in [
        "curl: (6) Could not resolve host: cursor.com",
        "curl: (28) Operation timed out after 15001 milliseconds",
        "curl: (22) The requested URL returned error: 500",
    ] {
        let failed = cursor_failed(&CurlError::from_stderr(stderr));
        assert_eq!(failed.error_kind(), Some(FuelError::Transient), "{stderr}");
        assert!(!failed.limits.needs_sign_in(), "{stderr}");
    }
    let limited = cursor_failed(&CurlError::from_stderr(
        "curl: (22) The requested URL returned error: 429",
    ));
    assert_eq!(limited.error_kind(), Some(FuelError::Transient));
    assert!(limited.rate_limited);
}

#[test]
fn curl_errors_carry_the_http_status() {
    assert_eq!(
        http_status("curl: (22) The requested URL returned error: 401 Unauthorized"),
        Some(401)
    );
    assert_eq!(
        http_status("curl: (22) The requested URL returned error: 403"),
        Some(403)
    );
    assert_eq!(http_status("curl: (7) Failed to connect"), None);
}

#[test]
fn codex_resumes_its_chat_only_with_a_plain_session_id() {
    assert_eq!(
        resume_line("codex", Some("0199a8f2-77e1-7c13-b5d6-1f2e3a4b5c6d")),
        "codex resume 0199a8f2-77e1-7c13-b5d6-1f2e3a4b5c6d"
    );
    for odd in ["x; rm -rf ~", "$(reboot)", "a b", "../x", ""] {
        assert_eq!(
            resume_line("codex", Some(odd)),
            "codex resume --last",
            "{odd}"
        );
    }
    assert_eq!(resume_line("codex", None), "codex resume --last");
    assert_eq!(resume_line("cursor", Some("abc")), "cursor-agent resume");
}

fn temp_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orcha-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create folder");
    dir
}

#[test]
fn a_config_that_is_not_json_is_left_alone() {
    let dir = temp_folder("bad-config");
    let path = dir.join("config.json");
    let broken = "{\"lang\": \"en\", \"accounts\": [";
    std::fs::write(&path, broken).expect("write config");
    let mut changed = false;
    let result = edit_config(&path, |_| {
        changed = true;
        Ok(())
    });
    assert!(result.is_err());
    assert!(
        !changed,
        "nothing is changed from a config that was not read"
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editing_the_config_keeps_other_settings_and_starts_a_missing_one() {
    let dir = temp_folder("edit-config");
    let path = dir.join("config.json");
    edit_config(&path, |config| add_to_config(config, "work")).expect("new config");
    assert!(std::fs::read_to_string(&path)
        .expect("read")
        .contains("claude-work"));
    std::fs::write(
        &path,
        r#"{"lang": "en", "accounts": [{"id": "claude-work", "kind": "claude"}]}"#,
    )
    .expect("write config");
    edit_config(&path, |config| remove_from_config(config, "claude-work")).expect("edit");
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(saved["lang"], "en");
    assert_eq!(saved["accounts"], serde_json::json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_claude_state_file_that_is_not_json_keeps_its_text() {
    let dir = temp_folder("bad-state");
    let path = dir.join(".claude.json");
    std::fs::write(&path, "not json").expect("write state");
    let profile = serde_json::json!({"emailAddress": "me@x.io"});
    assert!(set_profile(&path, &profile).is_err());
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "not json");
    assert!(set_profile(&dir.join("missing.json"), &profile).is_ok());
    assert!(!dir.join("missing.json").exists(), "Claude makes its own");
    let _ = std::fs::remove_dir_all(&dir);
}
