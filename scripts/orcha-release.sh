#!/usr/bin/env bash
# Cut an orcha release in one go:
#
#   scripts/orcha-release.sh 0.9.10          # or: just orcha-release 0.9.10
#
# 1. checks the tree is clean, on the orcha branch and in sync with origin
# 2. writes docs/orcha/releases/<version>.md when it is missing (install
#    lines from the last notes + this release's commit subjects) and opens
#    it in $EDITOR to tidy up
# 3. sets ORCHA_VERSION, runs fmt and a build check
# 4. commits "orcha: <version>", tags orcha-<version>, pushes both
# 5. follows the GitHub build (Mac arm64 + x86_64, Windows zip) to the end
#
# Flags: --yes (no questions), --no-edit (keep the notes as written),
#        --test (run the test suite too), --no-watch (do not follow CI).
set -euo pipefail

usage() {
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

version=""
assume_yes=0
edit_notes=1
run_tests=0
watch=1
for arg in "$@"; do
    case "$arg" in
        --yes | -y) assume_yes=1 ;;
        --no-edit) edit_notes=0 ;;
        --test) run_tests=1 ;;
        --no-watch) watch=0 ;;
        -h | --help) usage ;;
        -*) echo "unknown flag: $arg" >&2; usage 1 ;;
        *) version="$arg" ;;
    esac
done
[[ -n "$version" ]] || usage 1
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.2.3, got '$version'" >&2; exit 1; }

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
branch="orcha"
tag="orcha-$version"
app_file="src/orcha_app.rs"
notes="docs/orcha/releases/$version.md"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
confirm() {
    [[ $assume_yes == 1 ]] && return 0
    read -r -p "$1 [y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]]
}

current="$(sed -n 's/^.*const ORCHA_VERSION: &str = "\([0-9][0-9.]*\)";.*$/\1/p' "$app_file")"
[[ -n "$current" ]] || { echo "ORCHA_VERSION not found in $app_file" >&2; exit 1; }

say "orcha $current -> $version"

# newer than the current version
newest="$(printf '%s\n%s\n' "$current" "$version" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)"
if [[ "$version" == "$current" || "$newest" != "$version" ]]; then
    echo "$version is not newer than $current" >&2
    exit 1
fi

[[ "$(git branch --show-current)" == "$branch" ]] || { echo "switch to the $branch branch first" >&2; exit 1; }
if [[ -n "$(git status --porcelain)" ]]; then
    echo "the working tree has uncommitted changes; commit or stash them first" >&2
    git status --short >&2
    exit 1
fi
git fetch -q origin "$branch" --tags
if [[ -n "$(git rev-list "HEAD..origin/$branch")" ]]; then
    echo "origin/$branch has commits you do not have; pull first" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "tag $tag already exists" >&2
    exit 1
fi

# --- release notes
if [[ ! -s "$notes" ]]; then
    previous_tag="$(git tag -l 'orcha-*' --sort=-v:refname | head -1)"
    previous_notes="docs/orcha/releases/$current.md"
    say "writing $notes (commits since ${previous_tag:-the start})"
    {
        # keep the install block of the last notes: everything before "**New in"
        if [[ -s "$previous_notes" ]]; then
            awk '/^\*\*New in /{exit} {print}' "$previous_notes" | sed "s/$current/$version/g"
        fi
        echo "**New in $version**"
        git log --no-merges --format='%s' ${previous_tag:+"$previous_tag..HEAD"} |
            grep -E '^(fix|feat|perf|install|orcha)(\(.*\))?:' |
            sed -E 's/^[a-z]+(\([^)]*\))?: */- /' || true
    } >"$notes"
fi
if [[ $edit_notes == 1 && -t 0 ]]; then
    "${EDITOR:-vi}" "$notes"
fi
[[ -s "$notes" ]] || { echo "$notes is empty" >&2; exit 1; }
echo "----- $notes"
cat "$notes"
echo "-----"

# --- version and checks
say "setting ORCHA_VERSION = $version"
sed -i.bak "s/const ORCHA_VERSION: &str = \"$current\";/const ORCHA_VERSION: \&str = \"$version\";/" "$app_file"
rm -f "$app_file.bak"
grep -q "const ORCHA_VERSION: &str = \"$version\";" "$app_file" || { echo "failed to set ORCHA_VERSION" >&2; exit 1; }

clean_env=(env -u ORCHA_APP -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_ENV -u HERDR_PANE_ID)
say "cargo fmt --check"
cargo fmt --check
say "cargo check"
"${clean_env[@]}" cargo check --quiet
if [[ $run_tests == 1 ]]; then
    say "cargo nextest run"
    "${clean_env[@]}" cargo nextest run
fi

git --no-pager diff --stat
if ! confirm "commit, tag $tag and push to origin?"; then
    echo "stopped; the version bump and notes are left in the working tree"
    exit 1
fi

# --- commit, tag, push
git add "$app_file" "$notes"
git commit -q -m "orcha: $version"
git tag -a "$tag" -m "orcha $version"
say "pushing $branch and $tag"
git push -q origin "$branch"
git push -q origin "$tag"

# the repo we pushed to; `gh repo view` would pick the upstream herdr repo
repo="$(git remote get-url origin | sed -E 's#^(https://github\.com/|git@github\.com:)##; s#\.git$##')"
if [[ $watch == 0 ]]; then
    echo "pushed; the build runs at https://github.com/$repo/actions"
    exit 0
fi

# --- follow the build
say "waiting for the release build to start"
run_id=""
for _ in $(seq 1 30); do
    run_id="$(gh run list -R "$repo" --workflow orcha-release.yml --branch "$tag" -L 1 \
        --json databaseId -q '.[0].databaseId' 2>/dev/null || true)"
    [[ -n "$run_id" ]] && break
    sleep 4
done
if [[ -z "$run_id" ]]; then
    echo "could not find the build; see https://github.com/$repo/actions" >&2
    exit 1
fi
gh run watch "$run_id" -R "$repo" --exit-status
say "released: https://github.com/$repo/releases/tag/$tag"
gh release view "$tag" -R "$repo" --json assets -q '.assets[].name' | sed 's/^/  /'
