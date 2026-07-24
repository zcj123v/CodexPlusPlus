# Fork 更新源与全平台更新检查设计

日期：2026-07-23
分支：`linux-support`
状态：已批准

## 1. 背景与目标

当前 `linux-support` 分支的 Release 及 `latest.json` 由 `zcj123v/CodexPlusPlus` 发布，但客户端运行时更新检查仍固定读取：

```text
https://github.com/BigPizzaV3/CodexPlusPlus/releases/latest/download/latest.json
```

这使 fork 客户端无法发现 fork 自己发布的 Linux/Anthropic 版本。与此同时，项目主页、Issues、赞助、广告、主题市场、脚本市场和社区链接属于上游生态，不应随更新源迁移。

本次目标：

1. 先将 `upstream/main` 最新提交合入 `linux-support`。
2. Windows、macOS、Linux 客户端统一从 `zcj123v/CodexPlusPlus` 检查更新。
3. Windows/macOS 保持现有下载并启动安装包流程。
4. Linux 只展示新版本和安全下载入口，不自动执行 `pacman`、`dpkg` 或任何提权命令。
5. 所有非更新用途的 BigPizzaV3 链接保持不变。

## 2. 同步顺序

先执行：

```bash
git fetch upstream
git merge upstream/main
```

同步目标为当前 `linux-support`，不改写分支历史。解决冲突后运行受影响测试，再开始更新检查功能。同步完成后推送 `origin/linux-support`。

## 3. 更新源边界

在 `crates/codex-plus-core/src/update.rs` 中把更新源表达为更新专用常量：

```rust
pub const DEFAULT_UPDATE_REPOSITORY: &str = "zcj123v/CodexPlusPlus";
pub const DEFAULT_LATEST_JSON_URL: &str =
    "https://github.com/zcj123v/CodexPlusPlus/releases/latest/download/latest.json";
```

删除或替换当前未使用、容易误导的 `DEFAULT_REPOSITORY = "BigPizzaV3/CodexPlusPlus"`。不建立用户设置、环境变量或 CLI 参数覆盖；更新源固定为受信任的 fork Release，避免配置到任意不可信资产源。

`latest.json` 中的 `url` 和 `assets[].url` 继续由 Release workflow 的 `${{ github.repository }}` 生成，因此下载 URL自然指向 `zcj123v/CodexPlusPlus`。

### 3.1 必须保持为 BigPizzaV3 的内容

不得全局替换 `BigPizzaV3`。以下内容保持现状：

- Manager About 页的上游项目主页和 Issues。
- 注入菜单中的项目主页、Issues、Discord、Telegram。
- `Cargo.toml` 的 workspace repository 元数据。
- README、badges、issue template 与上游说明。
- `BigPizzaV3/Ad-List` 广告/赞助源及内置赞助链接。
- `BigPizzaV3/CodexPlusPlus-Themes` 主题市场。
- `BigPizzaV3/CodexPlusPlusScriptMarket` 脚本市场。
- Release notes 生成时用于比较差异的 `BigPizzaV3/CodexPlusPlus` upstream remote。

## 4. 全平台检查

### 4.1 公共流程

移除 `check_for_update` 在非 Windows/macOS 平台直接返回“无更新”的短路。所有平台统一执行：

```text
check_for_update(current_version)
  → GET DEFAULT_LATEST_JSON_URL
  → release_from_latest_json_payload
  → select_update_asset
  → is_newer_version
  → UpdateResult
```

HTTP 请求继续使用现有代理感知 client、状态码检查和 JSON 解码逻辑。

### 4.2 Windows 与 macOS

保持现有行为：

- Windows 选择当前架构安装器/zip 的既有排序逻辑。
- macOS 按 x64/arm64 选择 DMG。
- `perform_update` 下载 asset 到 app state 的 updates 目录，并启动安装器或打开 DMG。

### 4.3 Linux 资产选择

增加可独立测试的 Linux 发行版分类和资产排序：

- 读取 `/etc/os-release` 的 `ID` 与 `ID_LIKE`。
- `ID`/`ID_LIKE` 含 `arch`（包括 CachyOS）时优先非 debug 的 `.pkg.tar.zst`。
- `ID`/`ID_LIKE` 含 `debian` 或 `ubuntu` 时优先 `_amd64.deb`。
- 未识别发行版时，优先与当前架构相符的 Linux 安装资产；若无明确资产，则仍可展示 release 页面。
- x86_64 对应 release 资产命名中的 `x86_64`/`amd64`；本设计不宣称支持未发布的其他 Linux 架构。
- 必须排除名称含 `-debug-` 的 Arch debug package。

发行版识别失败不能使更新检查失败；它只降低资产选择精度。

## 5. Linux 更新操作

Linux 不执行自动安装，原因：

- `pacman -U`/`dpkg -i` 需要提权。
- 自动运行包管理器可能绕过用户的系统更新策略。
- Arch 与 Debian 的依赖处理和回滚语义不同。

Linux 检测到更新时：

- 显示 `latestVersion`、release notes、资产名和下载地址。
- 更新按钮打开选中的 fork asset URL；没有适配资产时打开 fork Release URL。
- 不写入 `/usr`，不调用 shell，不执行 `sudo`，不启动下载后的文件。
- UI 文案明确为“打开下载页面/下载安装包”，不能暗示已自动安装。

为保持边界清晰，core 的 Linux `perform_update` 返回一个结构化的“需要外部打开 URL”结果，或由 Tauri command 在 Linux 分支调用已有 `open_external_url`。具体接口在实施计划中以现有 `perform_update` 返回类型为准，避免重复 URL 打开逻辑。

## 6. 版本比较

fork Release tag 使用：

```text
v1.2.41-linux.1
v1.2.42-linux.1
```

版本比较必须满足：

- `1.2.42-linux.1 > 1.2.41`
- `1.2.42-linux.1 > 1.2.42`
- `1.2.42-linux.2 > 1.2.42-linux.1`
- 同一 tag 不提示更新。
- 仍兼容上游 `v1.2.42` 和历史普通 SemVer。

比较规则：先比较主数字段，再比较 fork 后缀。相同主版本时，无 `linux` 后缀视为基线，`linux.N` 按 N 递增。未知后缀保持现有宽松解析行为，但不得导致 panic。

## 7. 错误处理

- fork `latest.json` 网络失败、非 2xx、JSON 非法：保持现有错误传播和前端提示行为。
- 清单没有当前平台 asset：仍返回版本和 release URL，`asset_name/asset_url` 可为空。
- `/etc/os-release` 缺失或非法：回退通用 Linux 资产选择，不阻断检查。
- Linux 打开 URL 失败：返回明确错误，不尝试 shell fallback。
- 更新源不提供运行时配置覆盖，避免下载任意仓库资产。

## 8. 测试

### 8.1 Core

在 `crates/codex-plus-core/tests/updater.rs` 增加：

1. `DEFAULT_UPDATE_REPOSITORY == "zcj123v/CodexPlusPlus"`。
2. `DEFAULT_LATEST_JSON_URL` 精确指向 fork release 的 `latest.json`。
3. Arch/CachyOS 选择非 debug `.pkg.tar.zst`。
4. Debian/Ubuntu 选择 `_amd64.deb`。
5. 未识别 Linux 发行版合理回退。
6. 无 Linux asset 时仍保留 release URL。
7. fork tag 比较覆盖 `linux.1`/`linux.2` 和普通版本。
8. Linux 更新执行路径不调用包管理器或启动本地安装包。

为便于测试，OS release 解析接受字符串或路径参数；生产入口仍读取 `/etc/os-release`。

### 8.2 非更新链接防回归

增加轻量静态/单元断言，确认以下代表性 URL 仍指向 BigPizzaV3：

- Manager About 项目主页。
- Ads 默认源。
- Theme Market 默认源。
- Script Market 默认源。

该测试用于防止未来误用全局替换；不需要复制仓库全部外链。

### 8.3 现有回归

运行：

```bash
cargo test -p codex-plus-core --test updater
cargo test -p codex-plus-core --test ads
cargo test -p codex-plus-core
cd apps/codex-plus-manager && npm test && npm run check
```

已知与本功能无关的预存平台测试失败需单独记录，不可误判为本功能回归。

## 9. 完成标准

- `linux-support` 已包含当前 `upstream/main`。
- 客户端更新检查只请求 `zcj123v/CodexPlusPlus` 的 `latest.json`。
- Windows/macOS 现有更新行为不回归。
- Linux 能发现 fork 新版本并选择正确 Arch/deb 资产，但不会自动安装或提权。
- 赞助、广告、市场、上游主页和社区链接保持 BigPizzaV3。
- 单元/集成测试通过，分支推送到 `origin/linux-support`。
