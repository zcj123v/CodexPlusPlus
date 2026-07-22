# Linux deb 包与 Release notes 自动化实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 发布 GitHub Release 时自动构建并附加 deb 包，并向 release body 追加「与上游的差异 + 兼容性」说明。

**Architecture:** 在现有 `arch-package.yml` 增加与 Arch 平行的 `deb-package` job，复用 `build-binaries` 产物用 `dpkg-deb` 打包；在 `release-assets.yml` 增加 `release-notes` job，`git log upstream/main..tag` 生成差异列表，连同 `docs/release/compatibility.md` 用 `append_body` 追加到 release。

**Tech Stack:** GitHub Actions YAML、bash、dpkg-deb、softprops/action-gh-release@v2。

**Spec:** `docs/superpowers/specs/2026-07-22-linux-deb-release-design.md`（已批准）

## Global Constraints

- 不改 `Cargo.toml`、`package.json`、`.gitignore`。
- 不改任何 Rust/TS 源码；本计划只涉及 shell、YAML、Markdown。
- 本机（Arch）**没有 dpkg-deb、无 sudo**——本地验证只做语法/逻辑级检查，真实打包由 CI 验证。
- 兼容性措辞口径：**「可能可用，不保证」**，不是「保证可用」。
- deb 内容与 `scripts/installer/arch/PKGBUILD` 完全对齐（两二进制 + 两 desktop + 图标）。
- 仓库规范：注释尽量中文；提交信息沿用现有风格（如 `ci(arch): ...`）。
- `docs/superpowers/` 在 .gitignore 中但历史文件被跟踪，提交本计划用 `git add -f`。

---

### Task 1: debian 打包文件（control + build-package.sh）

**Files:**
- Create: `scripts/installer/debian/control`
- Create: `scripts/installer/debian/build-package.sh`

**Interfaces:**
- Consumes: `scripts/installer/arch/*.desktop`、`apps/codex-plus-manager/src-tauri/icons/icon.png`、workspace `Cargo.toml` 的 `version`。
- Produces: 脚本用法 `scripts/installer/debian/build-package.sh [--binaries-dir <dir>] [output-dir]`，输出 `<out>/codexplusplus_<version>_amd64.deb`；Task 2 的 CI job 以 `--binaries-dir pkg-bin dist/debian` 调用。

- [ ] **Step 1: 创建 control 模板**

写入 `scripts/installer/debian/control`（`@VERSION@` 由脚本注入，其余字段为最终值）：

```
Package: codexplusplus
Version: @VERSION@
Section: devel
Priority: optional
Architecture: amd64
Maintainer: zcj123v
Homepage: https://github.com/zcj123v/CodexPlusPlus
Depends: libwebkit2gtk-4.1-0, libayatana-appindicator3-1, libgtk-3-0, libglib2.0-0, libsoup-3.0-0, libgdk-pixbuf-2.0-0, libcairo2, libpango-1.0-0, libgcc-s1, hicolor-icon-theme
Description: External enhancement launcher and manager for the Codex desktop app (Linux port)
 Codex++ adds per-model context windows, relay profiles and provider
 management on top of the Codex desktop app.
```

注意：Description 续行必须以单个空格开头（deb 控制文件格式要求）。

- [ ] **Step 2: 创建构建脚本**

写入 `scripts/installer/debian/build-package.sh`：

```bash
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

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binaries-dir)
      binaries_dir="$2"
      shift 2
      ;;
    *)
      out_dir_arg="$1"
      shift
      ;;
  esac
done
out_dir="${out_dir_arg:-$repo_root/dist/debian}"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"
if [[ -z "$version" ]]; then
  echo "failed to read workspace version from Cargo.toml" >&2
  exit 1
fi

# 无预编译产物时本地全量构建（前端先构建，Tauri manager 编译期内嵌它）。
if [[ -z "$binaries_dir" ]]; then
  if [[ ! -d "$repo_root/apps/codex-plus-manager/dist" ]]; then
    (cd "$repo_root/apps/codex-plus-manager" && npm install --package-lock=false && npm run vite:build)
  fi
  (cd "$repo_root" && cargo build --release --workspace)
  binaries_dir="$repo_root/target/release"
fi

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
sed "s/^Version: @VERSION@$/Version: $version/" "$pkg_dir/control" > "$stage/DEBIAN/control"

mkdir -p "$out_dir"
package="$out_dir/codexplusplus_${version}_amd64.deb"
dpkg-deb --build --root-owner-group "$stage" "$package"

# 自检：control 元数据与包内文件清单。
dpkg-deb --info "$package" | grep -q "^ Package: codexplusplus$"
dpkg-deb --info "$package" | grep -q "^ Version: $version$"
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
```

然后 `chmod +x scripts/installer/debian/build-package.sh`。

- [ ] **Step 3: 本地语法与逻辑验证（无 dpkg-deb 环境）**

```bash
bash -n scripts/installer/debian/build-package.sh && echo "syntax ok"
# 验证版本读取与 control 注入逻辑（不调用 dpkg-deb）：
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)" && test -n "$version" && echo "version=$version"
sed "s/^Version: @VERSION@$/Version: $version/" scripts/installer/debian/control | grep "^Version: "
```

Expected: `syntax ok`；`version=1.2.41`（或当前 Cargo.toml 版本）；grep 输出 `Version: <版本>`。

- [ ] **Step 4: 条件性真实打包验证（仅当本机有 dpkg-deb）**

```bash
if command -v dpkg-deb >/dev/null; then
  mkdir -p /tmp/fake-bin && cp "$(command -v true)" /tmp/fake-bin/codex-plus-plus && cp "$(command -v true)" /tmp/fake-bin/codex-plus-plus-manager
  scripts/installer/debian/build-package.sh --binaries-dir /tmp/fake-bin /tmp/deb-out
else
  echo "dpkg-deb 不可用，真实打包留给 CI 验证"
fi
```

Expected: 有 dpkg-deb 时输出 `package written to /tmp/deb-out/codexplusplus_<version>_amd64.deb`；否则打印跳过信息（当前环境预期走 else 分支）。

- [ ] **Step 5: Commit**

```bash
git add scripts/installer/debian/control scripts/installer/debian/build-package.sh
git commit -m "ci(deb): add Debian packaging script and control template"
```

---

### Task 2: arch-package.yml 增加 deb-package job

**Files:**
- Modify: `.github/workflows/arch-package.yml`

**Interfaces:**
- Consumes: Task 1 的 `scripts/installer/debian/build-package.sh --binaries-dir` 用法；现有 `build-binaries` job 的 `linux-binaries` artifact。
- Produces: artifact `codexplusplus-deb-package`（`dist/debian/*.deb`）；release 事件时把 deb 附加到 release。

- [ ] **Step 1: 修改 workflow name 与 push paths**

`.github/workflows/arch-package.yml` 第 1 行：

```yaml
name: Linux packages
```

push paths 中 `'scripts/installer/arch/**'` 改为：

```yaml
      - 'scripts/installer/**'
```

- [ ] **Step 2: 追加 deb-package job**

在 `arch-package` job 之后（文件末尾）追加：

```yaml
  deb-package:
    name: Build Debian package (.deb)
    runs-on: ubuntu-latest
    needs: build-binaries
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Download Linux binaries
        uses: actions/download-artifact@v4
        with:
          name: linux-binaries
          path: pkg-bin

      - name: Build Debian package
        run: |
          chmod +x pkg-bin/codex-plus-plus pkg-bin/codex-plus-plus-manager
          scripts/installer/debian/build-package.sh --binaries-dir pkg-bin dist/debian

      - name: Upload Debian package
        uses: actions/upload-artifact@v4
        with:
          name: codexplusplus-deb-package
          path: dist/debian/*.deb
          if-no-files-found: error

      - name: Attach package to release
        if: github.event_name == 'release'
        uses: softprops/action-gh-release@v2
        with:
          files: dist/debian/*.deb
```

（`chmod` 必需：download-artifact 不保留可执行位。）

- [ ] **Step 3: YAML 语法验证**

```bash
npx --yes js-yaml .github/workflows/arch-package.yml > /dev/null && echo "yaml ok"
```

Expected: `yaml ok`。

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/arch-package.yml
git commit -m "ci(deb): build and attach Debian package in Linux packages workflow"
```

---

### Task 3: 兼容性声明 + release-notes job

**Files:**
- Create: `docs/release/compatibility.md`
- Modify: `.github/workflows/release-assets.yml`

**Interfaces:**
- Consumes: 无（独立任务）。
- Produces: `release-notes` job 读取 `docs/release/compatibility.md` 并 `append_body` 到 release；`latest-json` job 生成的 `body` 会带上该内容。

- [ ] **Step 1: 创建兼容性声明**

写入 `docs/release/compatibility.md`（口径：可能可用，不保证）：

```markdown
- 本分支面向社区 Linux 构建版 Codex Desktop；官方桌面版及其他版本可能可用，但不保证
- 提供 Linux x86_64 构建（Arch .pkg.tar.zst 与 Debian .deb）；其他系统或架构可能可用，但不保证
```

- [ ] **Step 2: release-assets.yml 追加 release-notes job**

在 `windows-installer` job 之前（文件 `jobs:` 之后）插入：

```yaml
  release-notes:
    name: Append fork notes to release
    runs-on: ubuntu-latest
    steps:
      - name: Checkout release tag
        uses: actions/checkout@v4
        with:
          ref: ${{ github.event.release.tag_name }}
          fetch-depth: 0

      - name: Generate fork notes
        run: |
          set -euo pipefail
          git remote add upstream https://github.com/BigPizzaV3/CodexPlusPlus.git
          git fetch upstream main
          {
            echo
            echo "---"
            echo
            echo "## 与上游 BigPizzaV3/CodexPlusPlus 的差异"
            echo
            diff_lines="$(git log --no-merges --oneline upstream/main..HEAD)"
            if [[ -n "$diff_lines" ]]; then
              echo '```'
              echo "$diff_lines"
              echo '```'
            else
              echo "无差异"
            fi
            echo
            echo "## 兼容性"
            echo
            cat docs/release/compatibility.md
          } > fork-notes.md
          cat fork-notes.md

      - name: Append notes to release
        uses: softprops/action-gh-release@v2
        with:
          body_path: fork-notes.md
          append_body: true
```

注意 YAML 缩进：job key 与 `windows-installer` 同级（两空格），step 缩进与现有 job 一致。

- [ ] **Step 3: 本地 dry-run notes 生成逻辑**

```bash
set -euo pipefail
diff_lines="$(git log --no-merges --oneline upstream/main..HEAD)"
test -n "$diff_lines" && echo "$diff_lines" | head -5
cat docs/release/compatibility.md
```

Expected: 输出 fork 独有提交列表（含 anthropic/linux 提交）与两条兼容性声明。
（本机已有 `upstream` remote，无需 add/fetch；CI 里由 job 内的 add/fetch 保证。）

- [ ] **Step 4: YAML 语法验证**

```bash
npx --yes js-yaml .github/workflows/release-assets.yml > /dev/null && echo "yaml ok"
```

Expected: `yaml ok`。

- [ ] **Step 5: Commit**

```bash
git add docs/release/compatibility.md .github/workflows/release-assets.yml
git commit -m "ci(release): append upstream-diff and compatibility notes to releases"
```

---

### Task 4: 推送并验证 CI

**Files:** 无（验证任务）。

**Interfaces:**
- Consumes: Task 1-3 全部产物。

- [ ] **Step 1: 推送**

```bash
git push origin linux-support
```

注意：push paths 已含 `scripts/installer/**`、`.github/workflows/arch-package.yml`，本次推送会触发 Linux packages workflow。

- [ ] **Step 2: 观察 CI**

```bash
gh run list --repo zcj123v/CodexPlusPlus --branch linux-support --limit 3
# 等最新 run 结束后：
gh run view <run-id> --repo zcj123v/CodexPlusPlus
```

Expected: `build-binaries`、`arch-package`、`deb-package` 三个 job 全部 success；`deb-package` 产出 artifact `codexplusplus-deb-package`。

- [ ] **Step 3: 下载 deb artifact 验证**

```bash
gh run download <run-id> --repo zcj123v/CodexPlusPlus --name codexplusplus-deb-package --dir /tmp/deb-verify
ls /tmp/deb-verify/*.deb
```

Expected: 存在 `codexplusplus_<version>_amd64.deb`（CI 脚本内已做 `--info/--contents` 自检，此处确认 artifact 真实存在即可）。

- [ ] **Step 4: release 效果说明（无需操作）**

release 附加与 notes 追加只在真实发布时触发，首次发布时人工检查 release 页面：
- assets 含 `.pkg.tar.zst` 与 `.deb`；
- body 末尾有「与上游的差异」「兼容性」两节。
