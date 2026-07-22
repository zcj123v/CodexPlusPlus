#!/usr/bin/env bash
# Verify that package inputs are x86-64 ELF binaries and optionally enforce a GLIBC ceiling.
set -euo pipefail

max_glibc=""
if [[ "${1:-}" == "--max-glibc" ]]; then
  if [[ $# -lt 3 ]]; then
    echo "Usage: scripts/installer/debian/verify-amd64-elf.sh [--max-glibc <version>] <binary>..." >&2
    exit 2
  fi
  max_glibc="$2"
  shift 2
fi
if [[ $# -eq 0 ]]; then
  echo "Usage: scripts/installer/debian/verify-amd64-elf.sh [--max-glibc <version>] <binary>..." >&2
  exit 2
fi
if [[ -n "$max_glibc" && ! "$max_glibc" =~ ^GLIBC_[0-9]+([.][0-9]+)*$ ]]; then
  echo "error: invalid GLIBC ceiling: $max_glibc" >&2
  exit 2
fi

for binary in "$@"; do
  if [[ ! -f "$binary" ]]; then
    echo "error: required binary does not exist: $binary" >&2
    exit 1
  fi

  description="$(file -b "$binary")"
  if [[ "$description" != *"ELF 64-bit"* || "$description" != *"x86-64"* ]]; then
    echo "error: expected an x86-64 ELF binary: $binary ($description)" >&2
    exit 1
  fi
  echo "verified x86-64 ELF: $binary"

  if [[ -n "$max_glibc" ]]; then
    glibc_versions="$(objdump -T "$binary" | awk '
      {
        line = $0
        while (match(line, /GLIBC_[0-9]+([.][0-9]+)*/)) {
          print substr(line, RSTART, RLENGTH)
          line = substr(line, RSTART + RLENGTH)
        }
      }
    ' | sort -Vu)"
    if [[ -z "$glibc_versions" ]]; then
      echo "error: no GLIBC symbol versions found in $binary" >&2
      exit 1
    fi

    highest_glibc="$(printf '%s\n' "$glibc_versions" | tail -n 1)"
    highest_allowed="$(printf '%s\n%s\n' "$max_glibc" "$highest_glibc" | sort -Vu | tail -n 1)"
    if [[ "$highest_allowed" != "$max_glibc" ]]; then
      echo "error: $binary requires $highest_glibc, newer than allowed $max_glibc" >&2
      exit 1
    fi
    echo "verified GLIBC ceiling: $binary requires at most $highest_glibc (allowed $max_glibc)"
  fi
done
