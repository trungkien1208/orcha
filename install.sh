#!/bin/sh
# orcha in one line (macOS, Apple Silicon or Intel):
#   curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
# Installs the orcha app to ~/.local/bin/orcha and its extras (fuel and
# account scripts, hand-off, theme, logo font; extras/ in this repository)
# to ~/.orcha. Run it again to update.
#
# One question, asked once: add Easy Job, a pixel-art office for your
# agents? Default no. `sh -s -- --office` answers yes, `--no-office` no; a
# run without a terminal answers no.
#
# The app is checked against the release's SHA256SUMS.txt before it is
# put in place. `orcha update` names the release in ORCHA_INSTALL_TAG.
set -e

OFFICE=ask
for arg in "$@"; do
  case "$arg" in
    --office) OFFICE=yes ;;
    --no-office) OFFICE=no ;;
    *) echo "unknown option: $arg (use --office or --no-office)"; exit 2 ;;
  esac
done

if [ "$(uname)" != Darwin ]; then
  echo "orcha's ready-made build is for macOS (and Windows: see install.ps1)."
  echo "Other machines: build it from source, see https://github.com/trungkien1208/orcha#build-from-source"
  exit 1
fi
case "$(uname -m)" in
  arm64) ASSET=orcha-macos-arm64 ;;
  x86_64) ASSET=orcha-macos-x86_64 ;;
  *) echo "orcha has no ready-made build for $(uname -m) yet."; exit 1 ;;
esac

BIN="$HOME/.local/bin"
EXTRAS="${ORCHA_DIR:-$HOME/.orcha}"
TAG="${ORCHA_INSTALL_TAG:-}"
case "$TAG" in
  "") ;;
  orcha-*[!A-Za-z0-9._-]* | orcha-) echo "unknown release: $TAG"; exit 2 ;;
  orcha-*) ;;
  *) echo "unknown release: $TAG"; exit 2 ;;
esac
if [ -z "$TAG" ]; then
  # Ask GitHub which release is newest: the "latest/download" link can point
  # at the previous release for a few minutes after a new one is published.
  TAG=$(curl -fsSL --connect-timeout 5 --max-time 15 \
    https://api.github.com/repos/trungkien1208/orcha/releases/latest 2>/dev/null \
    | sed -n 's/.*"tag_name": *"\(orcha-[A-Za-z0-9._-]*\)".*/\1/p' | head -n 1)
fi
if [ -n "$TAG" ]; then
  BASE="https://github.com/trungkien1208/orcha/releases/download/$TAG"
else
  BASE="https://github.com/trungkien1208/orcha/releases/latest/download"
fi

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1"
  else
    openssl dgst -sha256 -r "$1"
  fi | cut -d' ' -f1
}

echo "🐋🍵 orcha"
echo "→ downloading the app"
mkdir -p "$BIN"
# Stage beside the destination so the final rename is atomic, even when
# the system temporary directory is on another filesystem.
tmp=$(mktemp "$BIN/.orcha-install.XXXXXX")
sums=
extras_tmp=
cleanup() {
  rm -f "$tmp"
  [ -z "$sums" ] || rm -f "$sums"
  [ -z "$extras_tmp" ] || rm -f "$extras_tmp"
}
trap cleanup EXIT
sums=$(mktemp)
curl -fL --progress-bar -o "$tmp" "$BASE/$ASSET"
curl -fsSL -o "$sums" "$BASE/SHA256SUMS.txt"
expected=$(awk -v f="$ASSET" '$2 == f || $2 == "*" f { print $1; exit }' "$sums")
if [ -z "$expected" ] || [ "$(sha256_of "$tmp")" != "$expected" ]; then
  echo "The download did not match the release's checksum, so nothing was installed."
  echo "Try again in a few minutes."
  exit 1
fi
chmod +x "$tmp"
# Rename the new inode over the old one: running processes keep theirs,
# and a failed rename leaves the current app in place.
mv -f "$tmp" "$BIN/orcha"
xattr -d com.apple.quarantine "$BIN/orcha" 2>/dev/null || true

echo "→ setting up the extras"
# The extras (themes, fonts, scripts) come with the same release as the app,
# checked against the same SHA256SUMS.txt. Releases before 0.14.1 have none.
extras_expected=$(awk '$2 == "orcha-extras.tar.gz" || $2 == "*orcha-extras.tar.gz" { print $1; exit }' "$sums")
if [ -z "$extras_expected" ]; then
  echo "  (this release has no extras; the app works without them)"
else
  extras_tmp=$(mktemp)
  extras_new="$EXTRAS.new.$$"
  curl -fsSL -o "$extras_tmp" "$BASE/orcha-extras.tar.gz"
  if [ "$(sha256_of "$extras_tmp")" != "$extras_expected" ]; then
    rm -f "$extras_tmp"
    echo "The extras did not match the release's checksum, so they were not changed."
    exit 1
  fi
  rm -rf "$extras_new"
  mkdir -p "$extras_new"
  tar -xzf "$extras_tmp" -C "$extras_new"
  rm -f "$extras_tmp"
  # Swap folders: the scripts other apps link to keep their path.
  if [ -e "$EXTRAS" ]; then
    rm -rf "$EXTRAS.old"
    mv "$EXTRAS" "$EXTRAS.old"
  fi
  mv "$extras_new" "$EXTRAS"
  rm -rf "$EXTRAS.old"
  if command -v python3 >/dev/null 2>&1; then
    python3 -I "$EXTRAS/lib/setup.py" --app
  else
    echo "  (python3 is missing: run 'orcha setup' after xcode-select --install)"
  fi
fi

# Easy Job: only when the question is still open (not answered, not added)
if "$BIN/orcha" __offer-easy-job pending 2>/dev/null; then
  if [ "$OFFICE" = ask ]; then
    OFFICE=no
    if [ -t 1 ] && { : </dev/tty; } 2>/dev/null; then
      printf "Add Easy Job, a pixel-art office for your agents? (y/N) "
      read -r answer </dev/tty || answer=
      case "$answer" in
        y|Y|yes|YES|Yes) OFFICE=yes ;;
      esac
    else
      # nobody to ask: no, and the question stays open
      OFFICE=skip
    fi
  fi
  case "$OFFICE" in
    yes)
      echo "→ adding Easy Job"
      "$BIN/orcha" __offer-easy-job yes || true
      ;;
    no) "$BIN/orcha" __offer-easy-job no || true ;;
  esac
fi

echo
echo "✓ done. Open a new terminal window, then run:  orcha"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "  (or right now:  $BIN/orcha)" ;;
esac
