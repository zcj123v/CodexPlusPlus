# Windows 端口排除范围兼容实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Windows excluded port range 覆盖默认 guard/helper 端口时，Manager、Launcher 与 Helper 自动选择可绑定端口，静默启动链不再因 `os error 10013` 退出。

**Architecture:** 候选与验证逻辑集中在 `crates/codex-plus-core/src/ports.rs`：Windows 下 `WSAEACCES` 与 `AddrInUse` 都按“候选不可用”处理，但 `AddrInUse`/`WouldBlock` 在 guard 场景仍立即终止以保留单实例语义。Helper 使用独立 bind 候选，effective port 沿现有 LaunchStatus 写回；Manager 通过 StatusStore 读取 effective helper port。

**Tech Stack:** Rust、tokio、fs2、anyhow、Tauri 2。

**Spec:** `docs/superpowers/specs/2026-07-24-windows-excluded-port-range-design.md`

## Global Constraints

- 不调用 `netsh`、PowerShell，不修改 Windows excluded port range。
- 不发上游 PR；实现局部、可替换，供未来上游 #1309 修复覆盖。
- 非 Windows 行为保持不变，所有 fallback 以 `cfg!(windows)`/注入 `is_windows` 门控。
- guard 的 already-running/stale recovery 语义不得改变：`AddrInUse` 或 `WouldBlock` 必须终止候选并交还现有调用方。
- `WSAEACCES`/`os error 10013` 在 guard/helper 都视为端口不可用并尝试下一候选。
- helper 的 `bind_host` 不是 `127.0.0.1` 时不做 fallback。
- 不改变 `LaunchStatus` serde 字段；effective helper port 复用现有 `helper_port` 字段。
- protocol proxy 启用时 helper port 必须保持 `DEFAULT_PROTOCOL_PROXY_PORT=57321`，不做 helper fallback（relay config 的 `base_url` 已在启动前生成）。
- 日志记录 requested/effective/attempts；不输出环境变量或凭据。

---

### Task 1: Core 端口候选与 helper bind 逻辑

**Files:**
- Modify: `crates/codex-plus-core/src/ports.rs`

**Interfaces:**
- Produces: `is_excluded_port_error(&std::io::Error) -> bool`、`ResilientGuardAcquisition { guard, requested_port, effective_port, attempts }`、`acquire_resilient_guard_with_port_fallback(u16)`、`HelperBindResult { listener, requested_port, effective_port, attempts }`、`bind_helper_loopback_with_fallback(requested, bind_host)`、`find_rebindable_loopback_port() -> Option<u16>`。
- Preserves: `acquire_resilient_loopback_port_guard_with` 逐端口语义、`LoopbackPortGuard` 结构、非 Windows 行为。

- [ ] **Step 1: 写 guard fallback 失败测试**

在 `ports.rs` tests 增加：

```rust
#[test]
fn excluded_guard_port_falls_back_to_next_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Mutex::new(0usize);
    let acquisition = acquire_resilient_guard_with_port_fallback_with(
        57745,
        true,
        temp.path(),
        |port, state_dir| {
            *calls.lock().unwrap() += 1;
            if port == 57745 {
                Err(std::io::Error::from_raw_os_error(10013))
            } else {
                acquire_resilient_loopback_port_guard_at(port, state_dir)
            }
        },
        || acquire_loopback_port_guard(0),
    ).unwrap();
    assert_eq!(acquisition.requested_port, 57745);
    assert_eq!(acquisition.effective_port, 57746);
    assert_eq!(acquisition.attempts, 2);
}

#[test]
fn addr_in_use_guard_port_does_not_try_later_candidates() {
    let calls = Mutex::new(0usize);
    let error = acquire_resilient_guard_with_port_fallback_with(
        57745,
        true,
        Path::new("/unused"),
        |_, _| {
            *calls.lock().unwrap() += 1;
            Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use"))
        },
        || panic!("must not bind ephemeral"),
    ).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    assert_eq!(*calls.lock().unwrap(), 1);
}
```

- [ ] **Step 2: 验证测试失败**

```bash
cargo test -p codex-plus-core ports::tests::excluded_guard_port
```

Expected: 函数不存在。

- [ ] **Step 3: 实现 guard 候选**

实现 `is_excluded_port_error`（`raw_os_error()==10013 || PermissionDenied`）、`ResilientGuardAcquisition`、`acquire_resilient_guard_with_port_fallback` 与注入内核。

语义：
- 非 Windows：单次 acquire。
- Windows 候选：`requested..requested+8`，最后 ephemeral；每个候选调用注入 acquire。
- `Ok` 立即返回。
- `AddrInUse`/`WouldBlock` 立即上抛，不继续。
- 其他错误继续；耗尽返回含 requested、attempts、10013 提示的错误。
- ephemeral 路径先 `bind_ephemeral()` 得端口，再对同端口走 acquire，保持锁文件语义。

- [ ] **Step 4: 写 helper bind 失败测试**

```rust
#[test]
fn excluded_helper_port_falls_back_to_next_candidate() {
    let result = bind_helper_loopback_with_fallback_with(
        57321,
        true,
        "127.0.0.1",
        |_, port| {
            if port == 57321 {
                Err(std::io::Error::from_raw_os_error(10013))
            } else {
                TcpListener::bind(("127.0.0.1", 0))
            }
        },
    ).unwrap();
    assert_eq!(result.requested_port, 57321);
    assert!(result.effective_port != 0);
    assert_eq!(result.attempts, 2);
}
```

- [ ] **Step 5: 实现 helper bind 候选与可重绑端口**

实现 `HelperBindResult`、`bind_helper_loopback_with_fallback(requested, bind_host)` 与注入内核；`AddrInUse` 和 excluded 错误继续，其他错误耗尽上抛。`find_rebindable_loopback_port()` bind 0、drop、验证同端口可再 bind，最多 3 次。

- [ ] **Step 6: 验证并提交**

```bash
cargo fmt -p codex-plus-core -- --check
cargo test -p codex-plus-core ports::tests

git add crates/codex-plus-core/src/ports.rs
git commit -m "feat(core): fall back from excluded Windows loopback ports"
```

---

### Task 2: Launcher 与 Manager guard 接线

**Files:**
- Modify: `apps/codex-plus-launcher/src/main.rs`
- Modify: `apps/codex-plus-manager/src-tauri/src/lib.rs`
- Modify: `apps/codex-plus-launcher/src/main.rs` 测试区
- Modify: `apps/codex-plus-manager/src-tauri/tests/windows_subsystem.rs`

**Interfaces:**
- Consumes: Task 1 的 `ResilientGuardAcquisition`、`acquire_resilient_guard_with_port_fallback`。
- Produces: 日志 `launcher.guard_port_fallback`、`manager.guard_port_fallback`、`manager.guard_fallback_failed`；already-running 语义不变。

- [ ] **Step 1: 修改 launcher guard 获取**

`try_acquire_single_instance_guard` 改调：

```rust
codex_plus_core::ports::acquire_resilient_guard_with_port_fallback(
    codex_plus_core::ports::launcher_guard_port(),
)
```

`acquire_single_instance_guard_with_retry` 携带 acquisition；`Ok` 且 effective != requested 时记录：

```json
{"event":"launcher.guard_port_fallback","detail":{"requested_guard_port":...,"effective_guard_port":...,"attempts":...}}
```

`fallback_path()` 现有日志保留；`AddrInUse`/`WouldBlock` 分支不改。

- [ ] **Step 2: 修改 manager guard 获取**

`acquire_single_instance_guard` 同样调用统一函数。effective != requested 记录 `manager.guard_port_fallback`；fallback lock 日志保留；`AddrInUse|WouldBlock` 仍 `manager.already_running`；其他错误记 `manager.guard_fallback_failed` 后保留匿名 bind(0) 兜底。

- [ ] **Step 3: 静态测试**

Launcher tests 断言源码含 `acquire_resilient_guard_with_port_fallback`、`launcher.guard_port_fallback`、`effective_guard_port`。
Manager windows_subsystem 断言含 `manager.guard_port_fallback`、`manager.guard_fallback_failed`、`manager.already_running`。

- [ ] **Step 4: 验证并提交**

```bash
cargo test -p codex-plus-launcher
cargo test -p codex-plus-manager windows_subsystem

git add apps/codex-plus-launcher/src/main.rs apps/codex-plus-manager/src-tauri/src/lib.rs apps/codex-plus-manager/src-tauri/tests/windows_subsystem.rs
git commit -m "fix(launcher): recover from excluded guard ports"
```

---

### Task 3: Helper effective port 与 protocol proxy 边界

**Files:**
- Modify: `crates/codex-plus-core/src/launcher.rs`
- Modify: `crates/codex-plus-core/tests/launcher.rs`

**Interfaces:**
- Consumes: Task 1 的 `bind_helper_loopback_with_fallback`。
- Produces: `LaunchHooks::start_helper(u16) -> Result<u16>` 返回 effective port；`helper.listening` 含 requested/effective/attempts；LaunchStatus/helper injection 使用 effective port；protocol proxy 保持 57321。

- [ ] **Step 1: 修改 trait 与调用点**

```rust
async fn start_helper(&self, helper_port: u16) -> anyhow::Result<u16>;
```

调用点：

```rust
let helper_port = hooks.start_helper(helper_port).await?;
helper_started = true;
```

后续变量自然使用 effective port。错误路径 `shutdown_helper(helper_port)` 同样收到 effective。

- [ ] **Step 2: 实现 DefaultLaunchHooks helper**

Windows 且 bind_host == `127.0.0.1` 时使用 `bind_helper_loopback_with_fallback`；listener `set_nonblocking(true)` 后 `tokio::net::TcpListener::from_std`。日志：

```json
{"helper_port": effective, "requested_helper_port": requested, "attempts": n, "bind_host": ..., "address": "http://..."}
```

非 Windows 保持原 tokio bind 行为并返回 requested。

- [ ] **Step 3: protocol proxy 边界**

现有代码在 proxy enabled 时强制 `DEFAULT_PROTOCOL_PROXY_PORT`。在该分支禁止 helper fallback：若 bind 失败返回明确错误“protocol proxy 需要 57321，请释放该端口或避开 Windows excluded port range”。不要动态改 relay config。

- [ ] **Step 4: 测试**

更新 `tests/launcher.rs` 的 fake hooks：`start_helper` 返回 `Ok(helper_port)`。新增 fake 返回 `requested+7` 的测试，断言 injection/status/LaunchHandle 使用 effective。新增 protocol proxy 分支静态或单元断言不做 fallback。

- [ ] **Step 5: 验证并提交**

```bash
cargo test -p codex-plus-core --test launcher
cargo test -p codex-plus-core

git add crates/codex-plus-core/src/launcher.rs crates/codex-plus-core/tests/launcher.rs
git commit -m "fix(helper): report effective loopback port on Windows"
```

---

### Task 4: Manager 读取 effective helper port

**Files:**
- Modify: `apps/codex-plus-manager/src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `StatusStore::load_latest().helper_port`。
- Produces: `effective_helper_port(requested: u16) -> u16`；Dream Skin live 命令使用该值；无状态时回退 requested。

- [ ] **Step 1: 写失败测试**

新增：

```rust
#[test]
fn effective_helper_port_prefers_latest_launch_status() { /* temp StatusStore, helper_port 57399 */ }
#[test]
fn effective_helper_port_falls_back_to_requested() { /* missing/none/zero */ }
```

- [ ] **Step 2: 实现并接线**

实现：

```rust
fn effective_helper_port(requested: u16) -> u16 {
    codex_plus_core::status::StatusStore::default()
        .load_latest().ok().flatten()
        .and_then(|status| status.helper_port)
        .filter(|port| *port != 0)
        .unwrap_or(requested)
}
```

在 `apply_dream_skin_live` 两个调用点替换 `request.helper_port`。LaunchRequest 展示/转发字段不动。

- [ ] **Step 3: 验证并提交**

```bash
cargo test -p codex-plus-manager effective_helper_port
cargo test -p codex-plus-manager

git add apps/codex-plus-manager/src-tauri/src/commands.rs
git commit -m "fix(manager): use effective helper port after fallback"
```

---

### Task 5: 全量验证、Windows 实机验证与推送

**Files:** 无。

- [ ] **Step 1: 全量测试**

```bash
cargo test -p codex-plus-core
cargo test -p codex-plus-launcher
cargo test -p codex-plus-manager
cd apps/codex-plus-manager && npm test && npm run check
git diff --check
```

Expected: 全绿；已有与本任务无关的 warning 可记录。

- [ ] **Step 2: 推送并等待 Linux packages CI**

```bash
git push origin linux-support
gh run list --repo zcj123v/CodexPlusPlus --branch linux-support --limit 3
```

- [ ] **Step 3: Windows 实机验证**

在 `zcj12@172.20.0.3`（当前 excluded range 覆盖默认端口）：
1. 安装新构建；
2. 启动 Manager；
3. 点击启动 Codex++；
4. 检查日志含 `manager.guard_port_fallback` 或 effective helper port；
5. 预期 silent launcher 不再因 10013 退出，Codex 正常启动。
