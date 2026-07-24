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

---

## Final whole-branch review 修复（commit 8d0d226）

范围：仅 `crates/codex-plus-core/src/ports.rs`（1 file changed, 85 insertions, 7 deletions）。

### I-1（Important）`find_available_loopback_port` 返回未验证的 bind-0 端口

- 修复：`find_available_loopback_port()` 内部直接复用 `find_rebindable_loopback_port()` 的「bind 0 → drop → 验证同端口可再 bind，最多 3 次」逻辑，找不到时返回 0，保持原 `u16` 签名语义（调用方将 0 视为 bind ephemeral）。
- 测试：新增 `find_rebindable_loopback_port_returns_verified_port` 与 `find_available_loopback_port_returns_verified_port`，用真实 loopback 验证返回端口释放后可再次绑定。

### M-1（Minor）`is_excluded_port_error` 为 pub 死代码

- 修复：可见性由 `pub fn` 降为私有；因仅被同模块单元测试引用，加 `#[cfg(test)]` 避免非测试构建 dead-code warning。不作为长期公开 API。原有测试保持不变。

### M-2（Minor）确定性候选 `wrapping_add` 回绕产生 port 0

- 修复：`acquire_resilient_guard_with_port_fallback_with` 与 `bind_helper_loopback_with_fallback_with` 的候选生成改用 `checked_add`；`requested + offset` 溢出（回绕到 0）时跳出确定性区间，直接进入 ephemeral 路径。port 0 永远不作为确定性候选，guard 锁语义与日志不再被破坏。
- 测试：新增 `helper_candidates_near_u16_max_never_wrap_to_port_zero` 与 `guard_candidates_near_u16_max_never_wrap_to_port_zero`（requested=65530），断言仅产生 6 个确定性候选（65530..=65535）、从不出现 port 0，且 ephemeral 路径成功（attempts=7，effective_port != 0）。

### 验证

- `cargo test -p codex-plus-core --lib ports::`：22 passed, 0 failed。
- `git diff --check`：通过。
- `cargo build -p codex-plus-core`：无 ports.rs 相关 warning（既有 launcher.rs 等警告为修复前已存在）。
