/// An agent CLI orcha can list as an account. `kind` is herdr's agent id.
#[derive(Clone, Copy)]
pub(crate) struct AgentKind {
    pub kind: &'static str,
    pub command: &'static str,
    pub name: &'static str,
    /// The shell line that signs in.
    pub(super) login: &'static str,
    /// The shell line that continues the last chat, when the CLI has one.
    pub(super) resume: Option<&'static str>,
    /// orcha reads its usage numbers (FUEL gauges).
    pub(super) fuel: bool,
    /// Files under the home folder that exist once it is signed in; none
    /// known means orcha cannot tell.
    pub(super) sign_in: &'static [&'static str],
}

pub(super) const fn agent(
    kind: &'static str,
    command: &'static str,
    name: &'static str,
    login: &'static str,
    resume: Option<&'static str>,
    sign_in: &'static [&'static str],
) -> AgentKind {
    AgentKind {
        kind,
        command,
        name,
        login,
        resume,
        fuel: false,
        sign_in,
    }
}

pub(super) const CLAUDE: AgentKind = AgentKind {
    fuel: true,
    ..agent(
        "claude",
        "claude",
        "Claude",
        "claude auth login",
        Some("claude --continue"),
        &[],
    )
};

pub(super) const CODEX: AgentKind = AgentKind {
    fuel: true,
    ..agent(
        "codex",
        "codex",
        "Codex",
        "codex login",
        Some("codex resume --last"),
        &[".codex/auth.json"],
    )
};

pub(super) const CURSOR: AgentKind = AgentKind {
    fuel: true,
    ..agent(
        "cursor",
        "cursor-agent",
        "Cursor",
        "cursor-agent login",
        Some("cursor-agent resume"),
        &[".cursor/cli-config.json"],
    )
};

pub(crate) const AGENTS: &[AgentKind] = &[
    CLAUDE,
    CODEX,
    CURSOR,
    agent(
        "gemini",
        "gemini",
        "Gemini",
        "gemini",
        None,
        &[".gemini/oauth_creds.json", ".gemini/google_accounts.json"],
    ),
    agent(
        "copilot",
        "copilot",
        "Copilot",
        "copilot",
        Some("copilot --continue"),
        &[".copilot/config.json"],
    ),
    agent(
        "opencode",
        "opencode",
        "OpenCode",
        "opencode auth login",
        Some("opencode --continue"),
        &[".local/share/opencode/auth.json"],
    ),
    agent(
        "amp",
        "amp",
        "Amp",
        "amp login",
        None,
        &[".local/share/amp/secrets.json"],
    ),
    agent(
        "qwen",
        "qwen",
        "Qwen",
        "qwen",
        None,
        &[".qwen/oauth_creds.json"],
    ),
    agent("droid", "droid", "Droid", "droid", None, &[]),
    agent("kimi", "kimi", "Kimi", "kimi", None, &[]),
    agent("grok", "grok", "Grok", "grok", None, &[]),
    agent("kiro", "kiro-cli", "Kiro", "kiro-cli login", None, &[]),
    agent("devin", "devin", "Devin", "devin", None, &[]),
    agent("cline", "cline", "Cline", "cline", None, &[]),
    agent("kilo", "kilo", "Kilo", "kilo", None, &[]),
    agent("qodercli", "qodercli", "Qoder", "qodercli", None, &[]),
    agent("agy", "agy", "Antigravity", "agy", None, &[]),
    agent("pi", "pi", "Pi", "pi", None, &[]),
    agent("hermes", "hermes", "Hermes", "hermes", None, &[]),
    agent("letta", "letta", "Letta", "letta", None, &[]),
];

/// Supported hand-off targets use the same canonical CLI/name metadata.
pub(crate) const HANDOFF_AGENTS: [(&str, &str, &str); 3] = [
    (CLAUDE.kind, CLAUDE.command, CLAUDE.name),
    (CODEX.kind, CODEX.command, CODEX.name),
    (CURSOR.kind, CURSOR.command, CURSOR.name),
];

pub(crate) fn agent_kind(kind: &str) -> Option<&'static AgentKind> {
    AGENTS.iter().find(|agent| agent.kind == kind)
}
