#!/bin/sh
# Cmd+, : orcha's settings page in the browser (the app that runs this pane,
# else whichever orcha is on PATH). The older terminal menu stays `orcha menu`.
exec "${HERDR_BIN_PATH:-orcha}" settings
