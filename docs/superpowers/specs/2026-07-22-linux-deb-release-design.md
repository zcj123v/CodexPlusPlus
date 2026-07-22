# Linux deb 包与 Release notes 自动化设计

日期：2026-07-22
分支：linux-support
状态：已批准（用户确认于 2026-07-22）

## 1. 背景与目标

`linux-support` 分支已有 Arch 包 CI（`.github/workflows/arch-package.yml`）：push 到
`main`/`linux-support` 构建 `.pkg.tar.zst` artifact，发布 GitHub Release 时自动附加到
release 页面。

本次新增两件事：

1. **deb 包**：发布 Release 时与 Arch 包同时构建并附加 `codexplusplus_<version>_amd64.deb`。
2. **Release notes 自动化**：发布时自动向 release body 追加「与上游的差异」和「兼容性」两节。

触发方式明确为：**发布 GitHub Release 时**（沿用现有 `release: [published]` 触发器）。
Arch 包的发布构建已存在，本设计不改动其行为，只新增 deb 与 notes。

## 2. deb 包构建

### 2.1 方案选型

采用 **dpkg-deb 手工打包，复用 CI 已编译的 linux-binaries 产物**（已批准）。

否决的备选：

- `cargo-deb`：需改 `Cargo.toml`（仓库规范要求非必需不动），且按 crate 拆成两个 deb，
  与「一个产品包」形态不符。
- Tauri bundler：只打 manager 不含 launcher CLI，且需重新完整编译，产物形态与现有发行不一致。

### 2.2 打包内容

与 `scripts/installer/arch/PKGBUILD` 完全对齐：

| 来源 | 包内路径 | 权限 |
| --- | --- | --- |
| `codex-plus-plus`（linux-binaries） | `/usr/bin/codex-plus-plus` | 0755 |
| `codex-plus-plus-manager`（linux-binaries） | `/usr/bin/codex-plus-plus-manager` | 0755 |
| `scripts/installer/arch/codex-plus-plus.desktop` | `/usr/share/applications/codex-plus-plus.desktop` | 0644 |
| `scripts/installer/arch/codex-plus-plus-manager.desktop` | `/usr/share/applications/codex-plus-plus-manager.desktop` | 0644 |
| `apps/codex-plus-manager/src-tauri/icons/icon.png` | `/usr/share/icons/hicolor/256x256/apps/codexplusplus.png` | 0644 |

### 2.3 control 文件

新增 `scripts/installer/debian/control` 模板，`Version` 字段由构建脚本从 workspace
`Cargo.toml` 读取后注入（与 arch 脚本 `sed` 注入 pkgver 同一模式）。

- `Package: codexplusplus`
- `Architecture: amd64`
- `Section: devel`
- `Maintainer: zcj123v`（Debian 惯例为 `Name <email>`，无公开邮箱时只写名字）
- `Homepage: https://github.com/zcj123v/CodexPlusPlus`
- `Depends`（由 Arch 依赖映射为 Debian 包名）：
  `libwebkit2gtk-4.1-0, libayatana-appindicator3-1, libgtk-3-0, libglib2.0-0, libsoup-3.0-0, libgdk-pixbuf-2.0-0, libcairo2, libpango-1.0-0, libgcc-s1, hicolor-icon-theme`
- `Description`：沿用 PKGBUILD 的 pkgdesc
  （External enhancement launcher and manager for the Codex desktop app (Linux port)）

### 2.4 构建脚本

新增 `scripts/installer/debian/build-package.sh`，模式对齐
`scripts/installer/arch/build-package.sh`：

```
scripts/installer/debian/build-package.sh [--binaries-dir <dir>] [output-dir]
```

- 默认输出 `dist/debian/codexplusplus_<version>_amd64.deb`。
- 不传 `--binaries-dir` 时本地全量构建（前端缺失则 npm build + `cargo build --release --workspace`），
  与 arch 脚本行为一致。
- CI 传 `--binaries-dir` 指向下载的 `linux-binaries` artifact，跳过编译。
- `set -euo pipefail`；打包后用 `dpkg-deb --info` 与 `--contents` 自检
  （断言两个二进制、两个 desktop 文件、图标均在包内）。

不引入 lintian（避免拖慢 CI，后续需要再加）。

### 2.5 CI 集成

`.github/workflows/arch-package.yml`：

- workflow `name` 改为 `Linux packages`（文件名不改，保留运行历史）。
- 新增 `deb-package` job，与 `arch-package` 平行（`needs: build-binaries`），
  `runs-on: ubuntu-latest`（自带 dpkg-deb，无需容器）：
  1. checkout
  2. 下载 `linux-binaries` artifact
  3. 运行 `scripts/installer/debian/build-package.sh --binaries-dir pkg-bin dist/debian`
  4. `upload-artifact`：`codexplusplus-deb-package`（`dist/debian/*.deb`）
  5. `release` 事件时 `softprops/action-gh-release@v2` 附加 `dist/debian/*.deb`
- PR 触发时同样构建（验证打包不坏），但不附加到任何 release。
- push 路径过滤器无需改动：`scripts/installer/**` 变更目前不在 paths 列表里，
  需把 `scripts/installer/arch/**` 放宽为 `scripts/installer/**`，
  使 deb 脚本/模板变更也触发构建。

## 3. Release notes 自动化

### 3.1 位置与触发

`.github/workflows/release-assets.yml` 新增 `release-notes` job，
`release: [published]` 触发，无 `needs`（最先跑，与 asset 上传 job 不冲突；
`latest-json` job 排在 windows/macos 之后，读取 body 时自然带上 notes）。

需要 `fetch-depth: 0`（取完整历史）并
`git remote add upstream https://github.com/BigPizzaV3/CodexPlusPlus.git && git fetch upstream main`。

### 3.2 notes 结构

用 `softprops/action-gh-release@v2` 的 `append_body: true` 追加到用户手写说明之后，
不覆盖已有 body：

```markdown

---

## 与上游 BigPizzaV3/CodexPlusPlus 的差异

<git log --no-merges --oneline upstream/main..<tag> 的输出；为空时写「无差异」>

## 兼容性

- 本分支面向社区 Linux 构建版 Codex Desktop；官方桌面版及其他版本可能可用，但不保证
- 提供 Linux x86_64 构建（Arch .pkg.tar.zst 与 Debian .deb）；其他系统或架构可能可用，但不保证
```

差异列表即 fork 独有提交，随每次上游同步自动缩短，正好反映「这个 fork 多出了什么」。

### 3.3 兼容性声明源文件

新增 `docs/release/compatibility.md`，内容即上面「兼容性」一节的两条。
notes job 读取该文件注入——以后调整口径只改这个文件，不碰 workflow。

措辞原则（用户明确）：**不是「保证可用」，而是「可能可用」**——声明面向/测试的目标，
其余环境可能可用但不保证。

## 4. 错误处理

- 构建脚本 `set -euo pipefail`；版本读取失败立即退出（沿用 arch 脚本的 `test -n` 模式）。
- CI 自检步骤失败即 job 失败，坏包不会进入 release。
- `upstream` fetch 失败时 notes job 失败并可见，不静默生成残缺 notes。
- notes 只追加（`append_body`），不覆盖用户手写内容；重复运行同一 release 会重复追加，
  属可接受边界（release 发布是一次性事件）。

## 5. 测试与验证

- `build-package.sh` 的 `dpkg-deb --info/--contents` 自检（脚本内置）。
- PR 触发路径：deb job 在 PR 上跑通即验证打包逻辑。
- workflow YAML 语法：本地用 `npx js-yaml` 或 python yaml 解析校验。
- 真实 release 附加效果只能在实际发布时验证，首次发布时人工检查 release 页面。
- 不改任何 Rust/TS 代码，无需跑 cargo/npm 测试。

## 6. 文件清单

新增：

- `scripts/installer/debian/control`
- `scripts/installer/debian/build-package.sh`
- `docs/release/compatibility.md`

修改：

- `.github/workflows/arch-package.yml`（name、push paths、deb-package job）
- `.github/workflows/release-assets.yml`（release-notes job）
