# Windows 端口排除范围兼容设计

日期：2026-07-24
分支：`linux-support`
状态：已批准

## 1. 背景与目标

Windows 可能通过 Docker Desktop、WSL2、WinNAT 或其他系统组件把连续 TCP 端口加入 excluded port range。处于排除段的端口即使没有进程占用，普通程序绑定 `127.0.0.1:<port>` 也会失败并返回 `WSAEACCES`（`os error 10013`）。

当前 `linux-support` 的默认端口段会落在该范围时：

- Manager 无法绑定 manager guard；
- Launcher 无法绑定 launcher guard，直接退出；
- Helper 即使继续启动，也可能无法绑定 `57321`。

目标是让 Codex++ 在 Windows 上自动避开不可绑定的 loopback 端口，不修改系统网络配置，不发上游 PR；未来上游 BigPizzaV3/CodexPlusPlus#1309 的正式修复合入后，可以用上游实现替换本临时逻辑。

## 2. 设计原则

- 不调用 `netsh`、PowerShell 或修改 excluded port range。
- 不把 `bind 127.0.0.1:0` 的首次返回端口直接视为可用。
- Windows 上 `WSAEACCES/10013` 与 `AddrInUse` 一样按“该端口不可绑定”处理，继续尝试候选端口。
- 非 Windows 行为保持不变。
- 实际使用的端口必须写入日志；Helper 实际端口必须写入共享状态，避免 Manager 继续硬编码 `57321`。
- 逻辑集中在 `crates/codex-plus-core/src/ports.rs`，调用方只做最小调整，方便后续由上游修复覆盖。

## 3. 端口候选与验证

在 `ports.rs` 新增统一候选逻辑：

```text
requested guard/helper port
→ env 指定端口
→ 用户名 hash offset 端口
→ 一段确定性的后续候选端口
→ bind 0 获取的候选端口
```

每个候选都实际执行 bind 验证。成功后返回 listener/local_addr；失败时：

- `AddrInUse`：视为端口占用，继续候选；
- `WSAEACCES`/`os error 10013`：视为系统排除，继续候选；
- 其他不可恢复 I/O 错误：记录并继续有限次数后返回最终错误。

`find_available_loopback_port()` 不再只返回 `bind 0` 的端口号；它也必须确认返回端口能再次绑定，避免 Windows 分配 excluded range 的端口。

## 4. Manager guard

Manager 启动时：

1. 优先使用现有 `manager_guard_port()`；
2. 如果不可绑定，通过统一候选逻辑选择可绑定 guard；
3. 记录：
   - `requested_guard_port`
   - `effective_guard_port`
   - 原始错误（尤其 `10013`）
4. 如果端口已有真实 Manager 监听，维持“聚焦已有窗口并退出”的语义；
5. 如果只是 excluded/不可绑定，不当作已有实例。

现有 `manager.guard_failed` 日志改为可区分：

- `manager.already_running`
- `manager.guard_port_fallback`
- `manager.guard_fallback_failed`

Manager 不应因为某端口被系统排除就错误判断“已有 Manager 正在运行”。

## 5. Launcher guard

Launcher 使用与 Manager 相同的统一候选逻辑：

1. 优先使用现有 `launcher_guard_port()`；
2. 不可绑定时选择可绑定 guard；
3. 对真实已有 launcher 继续走已有 instance / stale recovery 语义；
4. 对 excluded/不可绑定端口自动换端口，而不是直接退出；
5. 记录 requested 与 effective guard port。

Manager 与 Launcher 的 guard 仍是不同职责，不共享同一端口；若用户名 offset 造成相邻端口都不可用，各自独立回退。

## 6. Helper 端口

Launcher 启动 helper 时：

1. 优先使用请求端口（通常 57321）；
2. 如果 bind 返回 `AddrInUse` 或 `WSAEACCES`，通过统一候选逻辑选择可绑定端口；
3. 返回并传递 `effective_helper_port`，而不是继续使用 requested 值；
4. `latest-status.json` 和诊断日志记录 effective helper port；
5. bridge/helper HTTP 地址使用 effective port。

Manager 读取 helper 状态时优先共享状态中的 effective helper port；没有共享状态时仍回退当前请求值，保持旧版本兼容。

## 7. 错误处理与日志

- 所有候选都失败时返回包含 requested port、尝试数量、最后错误的明确错误。
- 对 `WSAEACCES` 增加一句说明：该端口可能位于 Windows excluded port range（通常是 Docker/WSL/WinNAT 导致）。
- 日志不得输出环境变量中的敏感值；端口和路径可以记录。

## 8. 测试

核心测试在 `crates/codex-plus-core/src/ports.rs` 现有测试基础上增加，通过注入 bind/can_connect 模拟：

1. requested 返回 `AddrInUse` → 选择下一可绑定端口。
2. requested 返回 `os error 10013` → 选择下一可绑定端口。
3. `bind 0` 返回 excluded 端口 → 继续验证并换端口。
4. 所有候选不可绑定 → 返回明确错误。
5. 非 Windows 现有行为不回归。

Launcher/Manager 层增加静态或小单元测试，确保调用统一候选逻辑且日志包含 requested/effective port。Windows 实机验证步骤：

1. 保留当前 excluded port range；
2. 启动 Manager；
3. 点击启动 Codex++；
4. 预期 `manager.guard_port_fallback`、launcher guard fallback 与 `helper.listening` 使用非 57321/57745/57746 的可绑定端口；
5. Codex 正常启动，bridge 状态返回 ok。

## 9. 与上游关系

本实现只推送到 fork `linux-support`，不向上游发 PR。若上游 #1309 后续提供正式修复，本 fork 的临时候选逻辑应允许被上游实现整体替换；不要在文档中把它定义为长期公开 API。

## 10. 完成标准

- Windows excluded port range 覆盖默认 guard/helper 时，Manager 和 Launcher 能自动选择可绑定端口。
- Helper 实际端口被写入共享状态，Manager bridge 不继续使用失败端口。
- `WSAEACCES` 不再导致 silent launcher 直接退出。
- 非 Windows 与 Windows 正常端口行为无回归。
- 指定单元测试和 Windows 实机验证通过。
