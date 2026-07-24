# Final Fix Report

## 结论

已修复最终 review 的唯一 Important：Linux manager 不再在 async Tauri command 内调用同步 `xdg-open.status()` 并可能无限阻塞。

当前实现使用 manager 的 Tokio 直接依赖，通过 `tokio::process::Command` 无 shell 启动 `xdg-open`，并以 10 秒 timeout 等待真实退出状态。正常退出时继续校验 exit status，因此不会退化为“仅 spawn 即误报成功”；超时时执行 `kill().await`，随后 `wait().await` 回收子进程，并向调用方返回包含超时秒数及清理结果的明确错误。

## 代码变更

- `apps/codex-plus-manager/src-tauri/Cargo.toml`
  - manager 新增 `tokio.workspace = true` 直接依赖。
  - workspace 已有 Tokio，且 feature 已包含本修复所需的 `process`、`time`、`macros` 与 multi-thread runtime，因此没有引入新版本或额外 feature。
- `apps/codex-plus-manager/src-tauri/src/commands.rs`
  - `open_url` 改为 async。
  - Linux 分支使用 `tokio::process::Command::new("xdg-open").arg(url).spawn()`，没有 shell。
  - `bounded_url_opener_wait` 将等待限制在指定 duration 内。
  - `wait_for_url_opener` 在 10 秒内取得并验证真实 exit status；超时时 kill + wait，避免僵尸进程，并返回明确错误。
  - `open_external_url` 改为 async 并 await opener；Linux `perform_update` 的既有 async 调用链只增加必要的 await。
  - Windows 仍调用既有 `codex_plus_core::windows_open_url`；macOS 仍直接 spawn 系统 `open`，没有改变平台行为或引入 shell。
- `Cargo.lock`
  - 记录 manager 对 workspace Tokio 的直接依赖。

## 测试覆盖

测试不启动浏览器：

- 保留纯 `ExitStatus` helper 测试，覆盖成功与非零退出状态。
- 新增纯 timeout helper 测试：
  - ready future 在 deadline 内返回 `Completed`；
  - pending future 在短 deadline 后返回 `TimedOut`。
- 因 command 变为 async，URL scheme 拒绝测试改为 Tokio test；非法 URL 在调用 opener 前失败，不启动浏览器。

## 验证结果

### Manager Rust

```bash
cargo test -p codex-plus-manager --lib
```

结果：`47 passed; 0 failed`。

另执行完整 manager package 测试：

```bash
cargo test -p codex-plus-manager
```

结果：lib tests `47 passed; 0 failed`；静态集成 tests `21 passed; 0 failed`；bin/doc targets 通过（0 tests）。

### Core updater

```bash
cargo test -p codex-plus-core --test updater
```

结果：`17 passed; 0 failed`。

### Manager frontend

```bash
cd apps/codex-plus-manager
npm test
npm run check
```

结果：Node test `37 passed; 0 failed`；TypeScript `tsc --noEmit` 通过。

### i18n

```bash
node tools/i18n-verify.mjs
```

结果：

```text
plain: 739 referenced, 739 translated
template: 57 referenced, 57 translated
Dictionary matches every t()/tf() call site exactly.
```

### 格式与静态检查

```bash
git diff --check
cargo fmt -p codex-plus-manager -- --check
```

结果：均通过。

全 workspace `cargo fmt --all -- --check` 仍会报告本修复范围外、提交前已存在的格式差异：

- `crates/codex-plus-core/src/plugin_marketplace.rs`
- `crates/codex-plus-core/src/watcher.rs`
- `crates/codex-plus-data/src/lib.rs`

未修改这些无关文件；本次涉及的 manager crate 格式检查通过。

编译期间仍会显示 core 中既有 unused/dead-code warnings，本修复没有新增相关 warning，也未扩展范围处理它们。

## 安全与行为保证

- 无 shell，URL 作为单一参数传给 `xdg-open`。
- 不以 spawn 成功代替 opener 成功；仍依据真实 exit status 返回结果。
- async runtime worker 不执行同步 `Child::wait`。
- timeout 后同时终止并回收 child，不留下由本调用持有的僵尸进程。
- 清理失败不会被吞掉，错误会分别报告 kill 和 wait 的结果。
- 未启动真实浏览器进行测试。
