use super::*;

#[test]
fn restart_line_resumes_in_the_account_folder() {
    if cfg!(windows) {
        return;
    }
    assert_eq!(
        claude_restart_line(Some("/a/work"), Some("0b46-77")),
        "CLAUDE_CONFIG_DIR='/a/work' claude --resume 0b46-77"
    );
    assert_eq!(
        claude_restart_line(None, None),
        "env -u CLAUDE_CONFIG_DIR claude"
    );
    assert_eq!(
        claude_restart_line(Some("/a/it's"), Some("bad; rm")),
        "CLAUDE_CONFIG_DIR='/a/it'\\''s' claude",
        "quotes stay quoted and an odd session id is dropped"
    );
}
