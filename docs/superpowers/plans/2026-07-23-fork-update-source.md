# Fork 更新源与全平台更新检查实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 同步上游 v1.2.42，并让 Windows、macOS、Linux 只从 `zcj123v/CodexPlusPlus` 检查更新，同时保留所有 BigPizzaV3 生态与赞助链接。

**Architecture:** 先将 `upstream/main` merge 进 `linux-support`，再扩展 core 更新模块的平台上下文、Linux asset 选择和 fork tag 比较。Tauri command 在 Linux 仅用 `xdg-open` 打开可信 HTTP(S) 下载/Release URL；Windows/macOS 继续走现有下载安装流程。Manager 增加 `releaseUrl` 并显示符合平台语义的按钮文案。

**Tech Stack:** Rust、serde_json、reqwest、Tauri 2、React/TypeScript、node:test。

**Spec:** `docs/superpowers/specs/2026-07-23-fork-update-source-design.md`

## Global Constraints

- 更新检查源固定为 `zcj123v/CodexPlusPlus`，不提供用户设置、环境变量或 CLI 覆盖。
- `DEFAULT_LATEST_JSON_URL` 必须精确为 `https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json`。
- 不得全局替换 `BigPizzaV3`；About、Issues、Discord、Telegram、赞助/广告、Theme Market、Script Market、README、Cargo repository 与 upstream remote 保持原值。
- Linux 只打开 HTTP(S) 下载或 Release 页面；不得调用 `sudo`、`pacman`、`dpkg`、shell 或启动下载后的文件。
- Arch/CachyOS 优先非 debug `.pkg.tar.zst`；Debian/Ubuntu 优先 `_amd64.deb`。
- fork tag 排序：同主版本时普通版为基线，`linux.N` 按 N 递增。
- Windows/macOS 现有下载安装行为不得回归。
- 先同步上游，再修改更新功能；不改写分支历史。

---

### Task 1: 同步 upstream/main 到 linux-support

**Files:** 上游 merge 所涉及文件（由 Git 决定）。

**Interfaces:**
- Consumes: remote `upstream/main`（当前 `657cd33`, v1.2.42）。
- Produces: `linux-support` 包含上游 15 个提交，后续任务在同步后的代码上实施。

- [ ] **Step 1: 记录状态并合并上游**

```bash
git status --short
git fetch upstream
git merge upstream/main --no-edit
```

Expected: 工作树合并前干净；产生 merge commit；若冲突，逐个保留 linux-support 的 Anthropic/Linux/Release CI 功能并吸收上游 v1.2.42 修改。

- [ ] **Step 2: 验证同步结果**

```bash
git merge-base --is-ancestor upstream/main HEAD
git diff --check HEAD^1..HEAD
cargo test -p codex-plus-data --test storage_adapter
cd apps/codex-plus-manager && npm test && npm run check
```

Expected: ancestor 检查 0；diff check 无输出；storage、Node、tsc 全绿。

- [ ] **Step 3: Commit**

merge 成功时 Git 已生成 merge commit；记录：

```bash
git log --oneline -3
```

---

### Task 2: Core 更新源、fork 版本与 Linux asset 选择

**Files:**
- Modify: `crates/codex-plus-core/src/update.rs`
- Modify: `crates/codex-plus-core/tests/updater.rs`

**Interfaces:**
- Produces: `DEFAULT_UPDATE_REPOSITORY`、`DEFAULT_LATEST_JSON_URL`、`LinuxPackageFamily`、`UpdateOs`、`UpdateArch`、`UpdatePlatform`、`classify_linux_os_release`、`select_update_asset_for_platform`、`release_from_latest_json_payload_for_platform`；`UpdateCheck.release_url`。
- Preserves: `parse_version_tag`、`select_update_asset`、`release_from_latest_json_payload` 和 Windows/macOS 现有调用签名。

- [ ] **Step 1: 写更新源与 fork tag 失败测试**

在 `tests/updater.rs` 增加：

```rust
#[test]
fn update_source_points_to_fork() {
    assert_eq!(DEFAULT_UPDATE_REPOSITORY, "zcj123v/CodexPlusPlus");
    assert_eq!(DEFAULT_LATEST_JSON_URL,
        "https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json");
}

#[test]
fn fork_linux_revisions_sort_after_base_version() {
    assert!(is_newer_version("1.2.42-linux.1", "1.2.41").unwrap());
    assert!(is_newer_version("1.2.42-linux.1", "1.2.42").unwrap());
    assert!(is_newer_version("1.2.42-linux.2", "1.2.42-linux.1").unwrap());
    assert!(!is_newer_version("1.2.42-linux.1", "1.2.42-linux.1").unwrap());
    assert!(!is_newer_version("v1.2.42", "1.2.42").unwrap());
}
```

更新 imports 引入新常量。

- [ ] **Step 2: 验证测试先失败**

```bash
cargo test -p codex-plus-core --test updater update_source_points_to_fork
cargo test -p codex-plus-core --test updater fork_linux_revisions_sort_after_base_version
```

Expected: 新常量不存在或 URL 仍指向 BigPizzaV3；`1.2.42-linux.1 > 1.2.42` 失败。

- [ ] **Step 3: 实现更新常量与版本比较**

在 `update.rs`：

```rust
pub const DEFAULT_UPDATE_REPOSITORY: &str = "zcj123v/CodexPlusPlus";
pub const DEFAULT_LATEST_JSON_URL: &str =
    "https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json";

fn linux_revision(value: &str) -> Option<u64> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let (_, suffix) = normalized.split_once("-linux.")?;
    (!suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}
```

在 `is_newer_version` 主数字相等时比较 `linux_revision(...).unwrap_or(0)`。

- [ ] **Step 4: 写 os-release 与 asset 选择失败测试**

增加平台 helper 和测试：

```rust
fn linux(family: LinuxPackageFamily) -> UpdatePlatform {
    UpdatePlatform { os: UpdateOs::Linux, arch: UpdateArch::X86_64, linux_family: family }
}

#[test]
fn classifies_linux_package_families() {
    assert_eq!(classify_linux_os_release("ID=cachyos\nID_LIKE=arch\n"), LinuxPackageFamily::Arch);
    assert_eq!(classify_linux_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), LinuxPackageFamily::Debian);
    assert_eq!(classify_linux_os_release("NAME=Other\n"), LinuxPackageFamily::Unknown);
}

#[test]
fn linux_families_choose_native_non_debug_packages() {
    let assets = [
        "codexplusplus-debug-1.2.42-1-x86_64.pkg.tar.zst",
        "codexplusplus-1.2.42-1-x86_64.pkg.tar.zst",
        "codexplusplus_1.2.42_amd64.deb",
        "CodexPlusPlus-1.2.42-macos-x64.dmg",
    ].into_iter().map(|n| (n.to_string(), format!("https://example.test/{n}"))).collect::<Vec<_>>();
    assert_eq!(select_update_asset_for_platform(&assets, linux(LinuxPackageFamily::Arch)).unwrap().name,
        "codexplusplus-1.2.42-1-x86_64.pkg.tar.zst");
    assert_eq!(select_update_asset_for_platform(&assets, linux(LinuxPackageFamily::Debian)).unwrap().name,
        "codexplusplus_1.2.42_amd64.deb");
}
```

- [ ] **Step 5: 验证测试先失败**

```bash
cargo test -p codex-plus-core --test updater classifies_linux_package_families
cargo test -p codex-plus-core --test updater linux_families_choose_native_non_debug_packages
```

Expected: 类型/函数不存在。

- [ ] **Step 6: 实现平台上下文和 Linux 选择**

在 `update.rs` 增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxPackageFamily { Arch, Debian, Unknown }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOs { Windows, Macos, Linux, Other }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateArch { X86_64, Aarch64, Other }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdatePlatform {
    pub os: UpdateOs,
    pub arch: UpdateArch,
    pub linux_family: LinuxPackageFamily,
}
```

实现 `UpdatePlatform::current()`、`classify_linux_os_release(contents)`、私有 path reader；将现有 `platform_asset_rank` 改为接受 `UpdatePlatform`，Linux 排除 debug/source/Windows/macOS 资产，并按 family 排序。保留 `select_update_asset(assets)` wrapper。

- [ ] **Step 7: 保留 release URL，并取消 Linux 短路**

给 `UpdateCheck` 增加：

```rust
pub release_url: String,
```

`check_for_update` 所有平台统一 fetch；结果写入 `release_url: release.url`。若 merge 上游后 Linux 短路已消失，只补字段和平台选择。

增加测试：清单只有 `source.zip` 时 `release_from_latest_json_payload_for_platform` 的 `release.url` 保留、asset 为空。

- [ ] **Step 8: 运行 core 测试并提交**

```bash
cargo fmt --check
cargo test -p codex-plus-core --test updater
cargo test -p codex-plus-core --test ads
cargo test -p codex-plus-core --lib

git add crates/codex-plus-core/src/update.rs crates/codex-plus-core/tests/updater.rs
git commit -m "feat(core): check fork releases and select Linux packages"
```

---

### Task 3: Tauri Linux 安全打开更新 URL

**Files:**
- Modify: `apps/codex-plus-manager/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: Task 2 的 `UpdateCheck.release_url`、现有 `Release`。
- Produces: `check_update` payload 的 `releaseUrl`；Linux `perform_update` 打开 asset/release URL；共享 `validate_external_http_url`；Linux `open_url` 使用 `xdg-open`。

- [ ] **Step 1: 写纯函数失败测试**

在 commands tests 增加：

```rust
#[test]
fn external_url_validation_accepts_http_and_rejects_local_schemes() {
    assert_eq!(validate_external_http_url(" https://example.test/pkg ").unwrap(), "https://example.test/pkg");
    assert!(validate_external_http_url("file:///etc/passwd").is_err());
    assert!(validate_external_http_url("javascript:alert(1)").is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_update_url_prefers_asset_then_release() {
    let mut release = test_release();
    assert_eq!(linux_update_url(&release).unwrap(), release.asset_url.as_deref().unwrap());
    release.asset_url = None;
    assert_eq!(linux_update_url(&release).unwrap(), release.url);
}
```

`test_release()` 构造完整 `codex_plus_core::update::Release`。

- [ ] **Step 2: 验证失败**

```bash
cargo test -p codex-plus-manager external_url_validation_accepts_http
cargo test -p codex-plus-manager linux_update_url
```

Expected: helper 不存在。

- [ ] **Step 3: 实现 URL 校验、Linux选择和 xdg-open**

实现：

```rust
fn validate_external_http_url(url: &str) -> anyhow::Result<&str> { /* trim + http(s) only */ }
#[cfg(target_os = "linux")]
fn linux_update_url(release: &Release) -> anyhow::Result<&str> { /* asset_url, then release.url */ }
```

拆分 `open_url`：Windows 用现有 helper；macOS `open`；Linux `xdg-open`。不使用 shell。

- [ ] **Step 4: 修改 check_update 与 perform_update**

`check_update` 的成功/失败 JSON 均增加 `releaseUrl`。

Linux `perform_update` 在调用 core 下载前单独 cfg 分支：校验 URL → `xdg-open` → 返回 `launched:false`、`openedUrl`、`progress:100`；Windows/macOS 保持 core 下载启动流程。

- [ ] **Step 5: 验证并提交**

```bash
cargo fmt --check
cargo test -p codex-plus-manager external_url_validation_accepts_http
cargo test -p codex-plus-manager linux_update_url
cargo test -p codex-plus-manager open_external_url

git add apps/codex-plus-manager/src-tauri/src/commands.rs
git commit -m "feat(manager): open fork update downloads safely on Linux"
```

---

### Task 4: Manager 更新 UI 与上游链接防回归

**Files:**
- Modify: `apps/codex-plus-manager/src/App.tsx`
- Modify: `apps/codex-plus-manager/src/i18n-en.ts`
- Create: `apps/codex-plus-manager/src/update-links.test.ts`

**Interfaces:**
- Consumes: Task 3 payload `releaseUrl`, Linux perform_update message。
- Produces: `UpdateResult.releaseUrl`；Linux 按钮“打开安装包下载/打开 Release 页面”；静态测试保证更新源为 fork、代表性生态链接仍为 BigPizzaV3。

- [ ] **Step 1: 写静态失败测试**

`update-links.test.ts` 读取 App、core update、ads、theme/script market 源文件，断言：

```ts
assert.match(updateSource, /zcj123v\/CodexPlusPlus\/releases\/latest\/download\/latest\.json/);
assert.match(appSource, /github\.com\/BigPizzaV3\/CodexPlusPlus/);
assert.match(adsSource, /BigPizzaV3\/Ad-List/);
assert.match(themeSource, /BigPizzaV3\/CodexPlusPlus-Themes/);
assert.match(scriptSource, /BigPizzaV3\/CodexPlusPlusScriptMarket/);
```

- [ ] **Step 2: 修改 UpdateResult 与 release 重建**

增加 `releaseUrl?: string`。`performUpdate` 在 `assetUrl || releaseUrl` 时可执行，重建 Release 时 `url: update.releaseUrl ?? ""`。

- [ ] **Step 3: 修改 Linux 文案与进度行为**

用 `navigator.userAgent` 检测 Linux。Linux 按钮按 asset 是否存在显示“打开安装包下载”或“打开 Release 页面”，操作中显示“正在打开下载页面…”，不伪造下载百分比；Windows/macOS 文案保持“下载并运行安装包”。

在 `i18n-en.ts` 补对应英文翻译，并确保字典与调用精确匹配。

- [ ] **Step 4: 验证并提交**

```bash
cd apps/codex-plus-manager
npm test
npm run check
cd ../..
node tools/i18n-verify.mjs

git add apps/codex-plus-manager/src/App.tsx apps/codex-plus-manager/src/i18n-en.ts apps/codex-plus-manager/src/update-links.test.ts
git commit -m "feat(manager): present Linux fork updates as safe downloads"
```

---

### Task 5: 全量验证与推送

**Files:** 无。

- [ ] **Step 1: 全量回归**

```bash
cargo test -p codex-plus-core --test updater
cargo test -p codex-plus-core --test ads
cargo test -p codex-plus-core
cargo test -p codex-plus-manager
cd apps/codex-plus-manager && npm test && npm run check
cd ../.. && node tools/i18n-verify.mjs
git diff --check upstream/main..HEAD
```

Expected: 全绿；如仅出现已证明存在于 merge-base 的平台测试失败，记录证据。

- [ ] **Step 2: 验证链接边界**

```bash
git grep -n 'zcj123v/CodexPlusPlus' -- crates/codex-plus-core/src/update.rs .github/workflows
npm --prefix apps/codex-plus-manager test -- --test-name-pattern='update source'
```

Expected: fork 只用于更新/发布；静态测试确认 BigPizzaV3 生态链接保留。

- [ ] **Step 3: 推送并检查 CI**

```bash
git push origin linux-support
gh run list --repo zcj123v/CodexPlusPlus --branch linux-support --limit 5
```

Expected: push 成功；Linux packages CI 触发并通过。
