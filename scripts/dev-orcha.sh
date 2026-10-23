#!/bin/sh
set -eu

# Always launch from the checkout, with ORCHA's own config and session identity.
cd -- "$(dirname -- "$0")/.."
exec env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID \
    ORCHA_APP=1 cargo run -- --session orcha-dev "$@"
