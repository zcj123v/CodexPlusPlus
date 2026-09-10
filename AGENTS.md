# AGENTS.md

本文件为 CodexPlusPlus fork 的工作规范，指导 agent 在本仓库工作。

## 项目概述

本仓库是 [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 的 fork，定位为「仅保留 Linux 适配」的下游分支：支持 Linux 上的社区/官方 codex desktop 布局、Arch/Debian 打包与发布、fork 自有更新通道。功能特性与上游保持一致；新增功能需求先评估能否直接提给上游，只在确属平台适配时才在 fork 落地。

## 仓库结构

- `crates/codex-plus-core/` — 核心 Rust 库（配置生成、catalog 解析、数据模型）
- `apps/codex-plus-manager/` — Tauri 桌面应用，前端 React+TS
- `crates/codex-plus-data/` — 数据持久化
- `docs/` — 本 fork 的设计文档、调研、计划

## 关键代码位置

- Linux 安装适配:`crates/codex-plus-core/src/install/linux.rs`
- 桌面布局/进程检测:`crates/codex-plus-core/src/app_paths.rs`、`crates/codex-plus-core/src/watcher.rs`
- fork 更新通道:`crates/codex-plus-core/src/update.rs`、`apps/codex-plus-manager/src-tauri/src/commands.rs`
- 打包与发布:`scripts/installer/arch/`、`scripts/installer/debian/`、`.github/workflows/arch-package.yml`、`.github/workflows/release-assets.yml`

## 安全规则

- 禁止批量删除、rm -rf、rmdir /s
- 删除只能单个文件，删除前确认
- 禁止 sudo、提权、curl | bash
- 禁止泄露密钥、.env、auth.json、config.toml 凭据
- 覆盖文件前确认
- 不擅自改 Cargo.toml、package.json、.gitignore（除非任务必需）

## 命令执行

- 执行 bash 命令前确认
- 不运行未知脚本、不擅自装依赖
- 测试用 cargo test，不另起工具链

## 编码规范

- 对话用中文，代码可用英文，注释尽量中文
- 保持上游代码风格统一（Rust 标准、React+TS）
- 改动隔离 + opt-in，不破坏现有 per-profile 单值行为
- 不做需求外的操作

## 测试约定

- 沿用上游 `#[test]` + tempfile 风格（见 `crates/codex-plus-core/tests/relay_config.rs`）
- 断言读 config.toml 文本，如 `assert!(config.contains("model_catalog_json"))`
- 改行为要同步改/加对应测试

## 与上游同步

- `upstream` = https://github.com/BigPizzaV3/CodexPlusPlus.git
- `origin` = 用户自己的 GitHub fork(待创建)
- 分支约定:`linux-support` 为主线,只承载 Linux 适配;临时工作分支用 `linux/<topic>`
- 定期 `git fetch upstream && git rebase upstream/main` 保持同步
- 非 Linux 适配的改动一律先提给上游,不在 fork 长期携带

## Fork 版本规则

- 逻辑版本必须使用 `<上游版本>-linux.<fork序号>`：例如上游为 `1.2.43`、fork 第 4 次发布时为 `1.2.43-linux.4`；Release tag 使用 `v1.2.43-linux.4`。
- 上游版本以 `upstream/main` 的 workspace `Cargo.toml` 为准；同一上游版本内只递增 `linux.<fork序号>`，切换到新的上游版本后从 `linux.1` 开始。
- Arch `pkgver` 不允许连字符，打包时会将逻辑版本显示为 `1.2.43.linux.4`；包末尾的 `-<pkgrel>` 仅表示 Arch 打包修订，不得作为 fork 版本序号。
- 新建 Release 前必须核对 `Cargo.toml`、Git tag 和发行说明中的逻辑版本一致，禁止把上游版本号当作 fork 版本递增。
