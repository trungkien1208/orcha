#!/bin/sh
# orcha in one line (macOS, Apple Silicon):
#   curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
# Installs the orcha app to ~/.local/bin/orcha and its extras (fuel and
# account scripts, hand-off, theme, logo font) to ~/.orcha. No questions.
# Run it again to update.
set -e

if [ "$(uname)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "orcha's ready-made build needs a Mac with Apple Silicon."
  echo "Other machines: build it from source, see https://github.com/trungkien1208/orcha#build-from-source"
  exit 1
fi
command -v git >/dev/null 2>&1 || { echo "orcha needs git first: xcode-select --install"; exit 1; }

BIN="$HOME/.local/bin"
EXTRAS="${ORCHA_DIR:-$HOME/.orcha}"
# Ask GitHub which release is newest: the "latest/download" link can point
# at the previous release for a few minutes after a new one is published.
TAG=$(curl -fsSL --connect-timeout 5 --max-time 15 \
  https://api.github.com/repos/trungkien1208/orcha/releases/latest 2>/dev/null \
  | sed -n 's/.*"tag_name": *"\(orcha-[A-Za-z0-9._-]*\)".*/\1/p' | head -n 1)
if [ -n "$TAG" ]; then
  URL="https://github.com/trungkien1208/orcha/releases/download/$TAG/orcha-macos-arm64"
else
  URL="https://github.com/trungkien1208/orcha/releases/latest/download/orcha-macos-arm64"
fi

echo "🐋🍵 orcha"
echo "→ downloading the app"
mkdir -p "$BIN"
tmp=$(mktemp)
curl -fL --progress-bar -o "$tmp" "$URL"
chmod +x "$tmp"
# remove first: copying over a running binary gets it killed by macOS
rm -f "$BIN/orcha"
mv "$tmp" "$BIN/orcha"
xattr -d com.apple.quarantine "$BIN/orcha" 2>/dev/null || true

echo "→ setting up the extras"
if [ -d "$EXTRAS/.git" ]; then
  # an old or diverged copy cannot be updated in place: keep it aside, start fresh
  if ! git -C "$EXTRAS" pull --ff-only --quiet origin main 2>/dev/null; then
    old="$EXTRAS.old-$(date +%Y%m%d%H%M%S)"
    mv "$EXTRAS" "$old"
    echo "  (old extras kept at $old)"
    git clone --depth 1 --quiet https://github.com/trungkien1208/orcha-config "$EXTRAS"
  fi
else
  git clone --depth 1 --quiet https://github.com/trungkien1208/orcha-config "$EXTRAS"
fi
python3 -I "$EXTRAS/lib/setup.py" --app

echo
echo "✓ done. Open a new terminal window, then run:  orcha"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "  (or right now:  $BIN/orcha)" ;;
esac
