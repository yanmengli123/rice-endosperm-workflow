#!/usr/bin/env bash
# Resolve the GitHub release title from a notes file.
# Prints `title=...` for GitHub Actions. Does not create or edit a release.
#
# Usage:
#   scripts/github_release_notes.sh <notes-file> <tag>
#   scripts/github_release_notes.sh --self-test
set -euo pipefail

usage() {
  echo "usage: $0 <notes-file> <tag>" >&2
  echo "       $0 --self-test" >&2
  exit 2
}

resolve_title() {
  local file="$1"
  local tag="$2"
  if [[ ! -f "$file" ]]; then
    echo "release notes not found: $file" >&2
    return 1
  fi
  local title
  title="$(
    sed -n 's/^<!--[[:space:]]*release-title:[[:space:]]*\(.*[^[:space:]]\)[[:space:]]*-->$/\1/p' "$file" \
      | head -n 1
  )"
  if [[ -z "$title" ]]; then
    title="$tag"
  fi
  printf 'title=%s\n' "$title"
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  printf '%s\n' '<!-- release-title: v1.5.0: Share -->' '# heading' >"$tmp/with.md"
  printf '%s\n' '<!-- release-title: v1.5.0: Share   -->' >"$tmp/spaces.md"
  printf '%s\n' '# heading only' >"$tmp/without.md"

  expect() {
    local got
    got="$(resolve_title "$1" "$2")"
    if [[ "$got" != "title=$3" ]]; then
      echo "expected title=$3 from $1, got $got" >&2
      exit 1
    fi
  }

  expect "$tmp/with.md" v1.5.0 "v1.5.0: Share"
  expect "$tmp/spaces.md" v1.5.0 "v1.5.0: Share"
  expect "$tmp/without.md" v1.9.0 "v1.9.0"
  if resolve_title "$tmp/missing.md" v1.0.0 >/dev/null 2>&1; then
    echo "expected missing notes to fail" >&2
    exit 1
  fi

  repo_notes=".github/release-notes/v1.5.0.md"
  if [[ -f "$repo_notes" ]]; then
    expect "$repo_notes" v1.5.0 "v1.5.0: Share"
  fi
  echo "ok"
  exit 0
fi

[[ $# -eq 2 ]] || usage
resolve_title "$1" "$2"
