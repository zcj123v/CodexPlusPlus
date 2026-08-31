# Codex++ 后端进程监督与自动重连设计

## 背景

当 Codex++ manager 仍在托盘运行，但它启动的 `codex-plus-plus` launcher 意外退出时，Codex 前端会显示后端未连接。当前 manager 通过 `spawn_companion` 启动 launcher 后丢弃 `Child` 句柄，既不会等待子进程，也不会知道子进程何时退出。Linux 上因此会留下 `codex-plus-plus` zombie 进程，托盘状态还可能保留旧的 `running` 状态。

本次排查确认：

- 当前 fork 分支为 `linux-support`，相对 `upstream/main` 存在独立的 Linux 打包、relay 和运行时改动。
- 上游已同步到 `2a918aa`（v1.2.56 之后）。上游提交 `acdf0ec` 只解决 macOS 重启时旧 helper socket 释放延迟，不覆盖本问题。
- 本地 `main` 已指向 `upstream/main`，并保留 `backup/main-pre-upstream-sync-20260831` 作为回退点；`linux-support` 不做整体 rebase。

## 目标

1. manager 能感知自己启动的 launcher 退出。
2. 每个 launcher 子进程都被等待和回收，不再产生 zombie。
3. Codex 目标仍存活且 launcher 是意外退出时，manager 自动有限重试并恢复连接。
4. 用户主动重启或退出时不触发重复自动启动。
5. 重试耗尽后，前端看到明确的后端断开状态和可手动重启提示。
6. 不改变现有 relay、端口 fallback、Codex 注入和跨平台启动语义。

## 非目标

- 不让 launcher 自己无限重启。
- 不用 systemd 作为唯一监督机制。
- 不整体 rebase 当前 `linux-support` 到上游。
- 不在本次工作中重构无关的 relay、协议代理或前端页面。
- 不自动修改用户的 `config.toml`、`auth.json` 或其他凭据文件。

## 架构

### manager 级 BackendSupervisor

在 manager 中新增 `BackendSupervisor`，负责当前 manager 启动的 `codex-plus-plus` launcher 生命周期：

- 启动 launcher 时保留 `Child` 句柄。
- 保存当前启动请求、当前子进程、启动 generation 和有限重试信息。
- 在后台任务或线程中调用 `Child::wait()`。
- 子进程退出后记录退出信息，并根据主动停止标志、Codex 存活状态和重试预算决定下一步。
- 同时只允许一个 launcher 被当前 supervisor 托管。

core 的安装/启动层提供返回子进程句柄的内部能力；现有其他 `spawn_companion` 调用保持兼容，避免扩大改动面。

### 状态机

```text
starting
   ├─ 启动成功 → running
   ├─ 意外退出且允许重试 → restarting → starting
   ├─ 意外退出但超过上限 → disconnected
   └─ manager 主动停止 → stopped
```

状态沿用 `latest-status.json` 和已有 `LaunchStatus` 结构，通过现有字符串状态保持兼容：

- `starting`：首次启动或手动重启正在进行。
- `running`：launcher 和桥接流程已经就绪。
- `restarting`：检测到意外退出，正在等待下一次启动。
- `disconnected`：意外退出且重试预算耗尽，等待用户处理。
- `failed`：启动流程本身失败，或无法建立有效运行状态。
- `stopped`：仅作为内部 supervisor 状态，不把用户主动退出误判为故障。

## 数据流

1. manager 收到启动或重启请求，记录原始 app path、debug port、helper port 和启动时间。
2. supervisor 注册该请求并递增 generation。
3. supervisor 启动 launcher，保存 `Child`；launcher 继续负责 Codex 启动、helper 绑定、注入和 watchdog。
4. supervisor 后台等待 `Child::wait()`。
5. launcher 退出时记录 PID、exit code 或 signal、generation、主动停止标志，以及 Codex 进程/CDP 探测结果。
6. 非主动退出且 Codex 仍存在时，写入 `restarting` 并按退避策略重新启动原始请求。
7. 新 launcher 进入 `running` 后清零连续失败计数，并使旧监控代次失效。
8. 达到重试上限后写入 `disconnected`，保留最后一次退出原因，前端显示后端断开和手动重启入口。

启动失败、状态写入失败和重试异常都必须写入现有诊断日志，不能静默吞掉 supervisor 的关键错误。

## 主动停止与并发控制

- manager 执行 `restart_codex_plus` 前先通知 supervisor 进入主动停止阶段，再执行现有的 launcher/Codex 停止流程。
- manager 退出前设置全局退出标志，supervisor 不再重启 launcher。
- 每次新启动或主动停止递增 generation；旧监控任务发现 generation 不匹配时只负责回收自身资源，不得修改新任务状态。
- 重试期间用户再次点击重启时取消旧重试链，采用最新请求并重新开始重试计数。
- 启动和停止操作使用现有的 relay 切换锁或新增的窄范围 supervisor 锁，禁止并发产生多个 launcher。

## 重试策略

- 统计窗口：5 分钟。
- 最大自动重试：3 次。
- 建议退避：1 秒、3 秒、8 秒；每次等待期间都允许主动停止或新请求取消。
- 新 launcher 成功达到 `running` 后清除连续失败计数。
- 如果 Codex 目标已不存在，不自动重启，把退出视为正常生命周期结束。
- 如果无法确认 Codex 是否存活，优先写入诊断日志并进入 `disconnected`，避免在不确定时产生重启循环。

Codex 存活判断结合现有 watcher 的进程探测和 debug/CDP 端口探测，不仅依赖单一端口。

## 前端行为

沿用现有概览轮询和 `resolveLaunchStatus`：

- `starting` 和 `restarting` 显示启动中，并保留当前请求的等待提示。
- `running` 和现有 `running_degraded` 维持成功语义。
- `disconnected` 显示后端已断开、最后错误原因和手动重启按钮。
- 当前请求的终态不能被旧 generation 的状态覆盖。

如果现有状态解析器把未知状态当作失败，应补充显式的 `restarting`、`disconnected` 分支和对应测试，同时更新中英文文案。

## 测试方案

### core/install

- 启动 API 能返回可等待的子进程句柄。
- 子进程退出后 `wait()` 能完成并回收资源。
- 现有 `spawn_companion` 调用和平台路径行为不回归。

### manager supervisor

使用可注入的 spawn、wait、Codex 探测和状态写入依赖，覆盖：

1. 正常退出且 Codex 已关闭时不重启。
2. 意外退出且 Codex 仍运行时在预算内重启。
3. 连续启动失败达到 3 次后停止并写入 `disconnected`；单次启动流程本身失败仍写入 `failed`。
4. manager 主动停止时不触发自动重启。
5. 新 generation 建立后，旧任务不能覆盖状态或再次拉起进程。
6. 重试退避期间的新手动重启会取消旧重试链。

### status/frontend

- `restarting` 处于等待态。
- `disconnected` 显示明确错误和手动重启入口。
- 旧请求的失败状态不会覆盖新请求的 `starting` 或 `running`。
- 保留并运行现有 launcher、relay、端口和 frontend 测试。

## 实施边界

优先在当前 `linux-support` 分支上实现，不把上游 v1.2.56 的大规模变更整体带入。只选择与 supervisor 或明确运行时稳定性直接相关的上游修复进行对照或移植。实现后先运行受影响 crate 的定向 `cargo test`，再运行完整 `cargo test` 和 manager 前端测试；任何无法运行的检查都必须在交付说明中明确标注。
