#!/usr/bin/env bash
# 把 Codex++ 打成 Debian 包（.deb）。
#
# 用法: scripts/installer/debian/build-package.sh [--binaries-dir <dir>] [output-dir]
#
# --binaries-dir: 指向预编译的 codex-plus-plus / codex-plus-plus-manager
#   所在目录（CI 使用，跳过 cargo/npm 构建）；不传则本地全量构建。
# 依赖: dpkg-deb；本地构建时另需 cargo、npm。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
pkg_dir="$repo_root/scripts/installer/debian"
binaries_dir=""
out_dir_arg=""
out_dir_set=false

usage() {
  echo "Usage: scripts/installer/debian/build-package.sh [--binaries-dir <dir>] [output-dir]" >&2
}

argument_error() {
  echo "error: $*" >&2
  usage
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binaries-dir)
      if [[ $# -lt 2 || "$2" == -* ]]; then
        argument_error "--binaries-dir requires a directory"
      fi
      binaries_dir="$2"
      shift 2
      ;;
    -*)
      argument_error "unknown option: $1"
      ;;
    *)
      if [[ "$out_dir_set" == true ]]; then
        argument_error "only one output-dir may be specified"
      fi
      out_dir_arg="$1"
      out_dir_set=true
      shift
      ;;
  esac
done
out_dir="${out_dir_arg:-$repo_root/dist/debian}"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml")"
version="${version%%$'\n'*}"
if [[ -z "$version" ]]; then
  echo "failed to read workspace version from Cargo.toml" >&2
  exit 1
fi
# Cargo prereleases sort before the corresponding final release in Debian.
deb_version="$("$pkg_dir/cargo-version-to-deb.sh" "$version")"

# 无预编译产物时本地全量构建（前端先构建，Tauri manager 编译期内嵌它）。
if [[ -z "$binaries_dir" ]]; then
  if [[ ! -d "$repo_root/apps/codex-plus-manager/dist" ]]; then
    (cd "$repo_root/apps/codex-plus-manager" && npm install --package-lock=false && npm run vite:build)
  fi
  (cd "$repo_root" && cargo build --release --workspace)
  binaries_dir="$repo_root/target/release"
fi

if [[ ! -d "$binaries_dir" ]]; then
  echo "error: binaries directory does not exist: $binaries_dir" >&2
  exit 1
fi
"$pkg_dir/verify-amd64-elf.sh" --max-glibc GLIBC_2.35 \
  "$binaries_dir/codex-plus-plus" \
  "$binaries_dir/codex-plus-plus-manager"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

# 打包内容与 scripts/installer/arch/PKGBUILD 完全对齐。
install -Dm755 "$binaries_dir/codex-plus-plus" "$stage/usr/bin/codex-plus-plus"
install -Dm755 "$binaries_dir/codex-plus-plus-manager" "$stage/usr/bin/codex-plus-plus-manager"
install -Dm644 "$repo_root/scripts/installer/arch/codex-plus-plus.desktop" \
  "$stage/usr/share/applications/codex-plus-plus.desktop"
install -Dm644 "$repo_root/scripts/installer/arch/codex-plus-plus-manager.desktop" \
  "$stage/usr/share/applications/codex-plus-plus-manager.desktop"
install -Dm644 "$repo_root/apps/codex-plus-manager/src-tauri/icons/icon.png" \
  "$stage/usr/share/icons/hicolor/256x256/apps/codexplusplus.png"

mkdir -p "$stage/DEBIAN"
sed "s/^Version: @VERSION@$/Version: $deb_version/" "$pkg_dir/control" > "$stage/DEBIAN/control"

mkdir -p "$out_dir"
package="$out_dir/codexplusplus_${deb_version}_amd64.deb"
dpkg-deb --build --root-owner-group "$stage" "$package"

# 自检：control 元数据与包内文件清单。
package_info="$(dpkg-deb --info "$package")"
grep -q "^ Package: codexplusplus$" <<<"$package_info"
grep -q "^ Version: $deb_version$" <<<"$package_info"
grep -q "^ Architecture: amd64$" <<<"$package_info"
grep -q "^ Maintainer: zcj123v <noreply@github.com>$" <<<"$package_info"
contents="$(dpkg-deb --contents "$package")"
for path in \
  ./usr/bin/codex-plus-plus \
  ./usr/bin/codex-plus-plus-manager \
  ./usr/share/applications/codex-plus-plus.desktop \
  ./usr/share/applications/codex-plus-plus-manager.desktop \
  ./usr/share/icons/hicolor/256x256/apps/codexplusplus.png; do
  grep -qF "$path" <<<"$contents" || { echo "missing $path in package" >&2; exit 1; }
done
echo "package written to $package"
