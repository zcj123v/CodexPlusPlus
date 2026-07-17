#!/usr/bin/env bash
# Build Codex++ release binaries and package them as an Arch Linux package.
#
# Usage: scripts/installer/arch/build-package.sh [output-dir]
#
# Requires: cargo, npm (only when the frontend dist is missing), makepkg.
# makepkg refuses to run as root — run this as a normal user with sudo
# available for --syncdeps.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
pkg_dir="$repo_root/scripts/installer/arch"
out_dir="${1:-$repo_root/dist/arch}"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"
if [[ -z "$version" ]]; then
  echo "failed to read workspace version from Cargo.toml" >&2
  exit 1
fi

# 1. Build the frontend (the Tauri manager embeds it at compile time).
if [[ ! -d "$repo_root/apps/codex-plus-manager/dist" ]]; then
  (cd "$repo_root/apps/codex-plus-manager" && npm install --package-lock=false && npm run vite:build)
fi

# 2. Build release binaries.
(cd "$repo_root" && cargo build --release --workspace)

# 3. Stage package sources.
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp "$repo_root/target/release/codex-plus-plus" "$stage/"
cp "$repo_root/target/release/codex-plus-plus-manager" "$stage/"
cp "$pkg_dir/codex-plus-plus.desktop" "$stage/"
cp "$pkg_dir/codex-plus-plus-manager.desktop" "$stage/"
cp "$repo_root/apps/codex-plus-manager/src-tauri/icons/icon.png" "$stage/codexplusplus.png"
cp "$pkg_dir/PKGBUILD" "$stage/"
sed -i "s/^pkgver=.*/pkgver=$version/" "$stage/PKGBUILD"

# 4. Build the package.
mkdir -p "$out_dir"
(cd "$stage" && makepkg --force --noconfirm --syncdeps)
mv "$stage"/codexplusplus-*.pkg.tar.zst "$out_dir"/
echo "package written to $out_dir"
