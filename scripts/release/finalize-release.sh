#!/usr/bin/env bash
# Wait for Linux release packages, then append generated notes without losing the current body.
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

max_attempts="${RELEASE_ASSET_MAX_ATTEMPTS:-30}"
poll_seconds="${RELEASE_ASSET_POLL_SECONDS:-10}"
if [[ ! "$max_attempts" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: RELEASE_ASSET_MAX_ATTEMPTS must be a positive integer" >&2
  exit 2
fi
if [[ ! "$poll_seconds" =~ ^[0-9]+$ ]]; then
  echo "error: RELEASE_ASSET_POLL_SECONDS must be a non-negative integer" >&2
  exit 2
fi

assets=""
for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  assets="$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name')"
  have_arch=false
  have_deb=false
  while IFS= read -r asset; do
    [[ "$asset" == *.pkg.tar.zst ]] && have_arch=true
    [[ "$asset" == *.deb ]] && have_deb=true
  done <<<"$assets"

  if [[ "$have_arch" == true && "$have_deb" == true ]]; then
    echo "Release assets ready on attempt $attempt/$max_attempts."
    break
  fi

  if ((attempt == max_attempts)); then
    echo "error: timed out waiting for both a .pkg.tar.zst and a .deb release asset after $max_attempts attempts" >&2
    echo "Last observed release assets:" >&2
    if [[ -n "$assets" ]]; then
      printf '%s\n' "$assets" >&2
    else
      echo "(none)" >&2
    fi
    exit 1
  fi

  echo "Waiting for Linux release assets (attempt $attempt/$max_attempts; arch=$have_arch, deb=$have_deb)..."
  sleep "$poll_seconds"
done

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
