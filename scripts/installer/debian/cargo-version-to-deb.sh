#!/usr/bin/env bash
# Convert a Cargo SemVer string to a Debian upstream version.
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo "Usage: scripts/installer/debian/cargo-version-to-deb.sh <cargo-version>" >&2
  exit 2
fi

printf '%s\n' "${1/-/\~}"
