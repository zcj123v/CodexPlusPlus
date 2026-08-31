# Codex++ 后端进程监督与自动重连设计

## 背景

Codex++ manager 仍在托盘运行时，如果它启动的 `codex-plus-plus` launcher 意外退出，Codex 前端会显示后端未连接。当前 manager 通过 `spawn_companion` 启动 launcher 后丢弃 `Child` 句柄，既不等待子进程，也无法感知退出；Linux 上还会留下 zombie 进程。

上游已同步到 `2a918aa`（v1.2.56 之后）。上游 `acdf0ec` 只修复 macOS helper 端口释放延迟，不覆盖本问题。由于 `linux-support` 含有独立的 Linux、relay 和运行时改动，本次不整体 rebase。

## 目标与范围

- manager 托管自己启动的 launcher，并在退出后回收子进程。
- Codex 仍在运行且 launcher 意外退出时，自动有限重试。
- 用户主动重启或退出时不误触发自动重启。
- 重试耗尽后沿用现有 `failed` 状态显示错误和手动重启入口。
- 不改变现有 relay、端口 fallback、Codex 注入和其他平台启动逻辑。

不使用无限重启或 systemd 作为唯一监督机制，也不修改用户配置和凭据文件。

## 设计

在 manager 中增加一个轻量 supervisor：

1. 启动 launcher 时保留 `Child` 句柄。
2. 在后台等待 `Child::wait()`，保证进程被回收，并记录退出原因。
3. 退出后检查 manager 是否主动停止，以及 Codex 进程/CDP 是否仍存在。
4. 如果是意外退出且 Codex 仍运行，写入现有 `starting` 状态并重新启动原请求。
5. 5 分钟内最多自动重试 3 次，间隔为 1 秒、3 秒、8 秒。
6. 重试成功后恢复现有 `running` 状态；重试耗尽则写入 `failed`，保留最后错误。

只监督当前 manager 启动并持有句柄的 launcher。manager 主动重启前先取消旧监督，manager 退出时禁止自动重启。使用一个简单的互斥状态避免同时启动多个 launcher；不引入独立的 generation 状态机或新的持久化字段。

core 启动层增加返回子进程句柄的内部 API，现有其他 `spawn_companion` 调用保持兼容。Codex 存活判断复用 watcher 的进程探测和 debug/CDP 端口探测。

## 前端行为

继续使用现有概览轮询和 `resolveLaunchStatus`：

- `starting`：初次启动或自动重试中。
- `running`、`running_degraded`：保持现有成功语义。
- `failed`：显示后端断开、最后错误和手动重启按钮。

不新增 `restarting`、`disconnected` 等持久化状态，避免扩大前后端状态协议。

## 测试

- core 启动 API 返回的子进程可被 `wait()` 回收。
- 正常退出且 Codex 已关闭时不重启。
- 意外退出且 Codex 仍在运行时会重试。
- 3 次重试失败后写入 `failed`，不再继续拉起。
- manager 主动重启或退出时不触发自动重启。
- 现有状态解析、launcher、relay、端口和 frontend 测试保持通过。

测试优先使用可注入的最小 fake spawn/wait 行为，不引入完整的进程模拟框架。

## 实施边界

只在当前 `linux-support` 分支实现必要改动，不整体带入上游 v1.2.56。完成后先运行受影响 crate 的定向 `cargo test`，再运行完整 `cargo test` 和 manager 前端测试；无法运行的检查需明确说明。
