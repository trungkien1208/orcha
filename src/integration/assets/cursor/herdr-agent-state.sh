#!/bin/sh
# managed by herdr; reinstalling the integration replaces this file.
# HERDR_INTEGRATION_ID=cursor
# HERDR_INTEGRATION_VERSION=3

[ "${1:-}" = "session" ] || exit 0
[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

python3 -c '
import json
import os
import socket
import sys
import time

try:
    payload = json.load(sys.stdin)
except Exception:
    raise SystemExit(0)

def first_text(*names):
    for name in names:
        value = payload.get(name)
        if isinstance(value, str) and value:
            return value
    return None

event = first_text("hook_event_name", "hookEventName")
if event not in (None, "sessionStart"):
    raise SystemExit(0)

# The conversation id names the chat and its transcript; session_id can be the CLI session.
session_id = first_text("conversation_id", "conversationId", "session_id", "sessionId")
if session_id is None:
    raise SystemExit(0)
transcript_path = first_text("transcript_path", "transcriptPath")
model = first_text("model")

seq = time.time_ns()
params = {
    "pane_id": os.environ["HERDR_PANE_ID"],
    "source": "herdr:cursor",
    "agent": "cursor",
    "seq": seq,
    "agent_session_id": session_id,
}
if transcript_path is not None:
    params["agent_session_path"] = transcript_path
if model is not None:
    params["model"] = model
request = json.dumps({
    "id": f"herdr:cursor:{seq}",
    "method": "pane.report_agent_session",
    "params": params,
})
try:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(0.5)
        client.connect(os.environ["HERDR_SOCKET_PATH"])
        client.sendall((request + "\n").encode())
        client.recv(4096)
except Exception:
    pass
' 2>/dev/null || true
