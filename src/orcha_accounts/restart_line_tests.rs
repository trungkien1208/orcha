use super::*;

#[test]
fn restart_line_resumes_in_the_account_folder() {
    use crate::platform::orcha_env_command_line as line;
    assert_eq!(
        claude_restart_line(Some("/a/work"), Some("0b46-77")),
        line(
            "CLAUDE_CONFIG_DIR",
            Some("/a/work"),
            "claude --resume 0b46-77"
        )
    );
    assert_eq!(
        claude_restart_line(None, None),
        line("CLAUDE_CONFIG_DIR", None, "claude")
    );
    assert_eq!(
        claude_restart_line(Some("/a/it's"), Some("bad; rm")),
        line("CLAUDE_CONFIG_DIR", Some("/a/it's"), "claude"),
        "an odd session id is dropped"
    );
}
