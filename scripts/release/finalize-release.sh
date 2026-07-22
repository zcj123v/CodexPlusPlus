#!/usr/bin/env bash
# Append generated notes without losing the current release body.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: scripts/release/finalize-release.sh <notes-file>" >&2
  exit 2
fi

notes_file="$1"
: "${REPO:?REPO must be set}"
: "${TAG:?TAG must be set}"

if [[ ! -f "$notes_file" ]]; then
  echo "error: notes file does not exist: $notes_file" >&2
  exit 2
fi

marker='<!-- codexplusplus-fork-notes -->'
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
current_body="$work_dir/current-body.md"
updated_body="$work_dir/updated-body.md"

gh release view "$TAG" --repo "$REPO" --json body --jq .body >"$current_body"
if grep -Fq "$marker" "$current_body"; then
  echo "Fork notes marker already exists; release body is unchanged."
  exit 0
fi

cp "$current_body" "$updated_body"
cat "$notes_file" >>"$updated_body"
gh release edit "$TAG" --repo "$REPO" --notes-file "$updated_body"
echo "Fork notes appended while preserving the existing release body."
