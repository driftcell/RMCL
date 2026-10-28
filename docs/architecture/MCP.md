# Model Context Protocol（MCP）

状态：第二阶段实例工作流已实现；内容写入、设置页、权限分级和跨进程修改锁尚未实现

日期：2026-10-28

## 边界

`rmcl-mcp` 只负责协议、传输和稳定的输入输出模型，不重新实现安装、实例、内容或游戏逻辑。`rmcl-app::AppContext` 是 GUI 与外部协议共享的 headless 服务层，包含 Tokio runtime、Java、版本目录、内容安装器、Modrinth、更新服务和同一个 `TaskManager`。

- GPUI 只存在于 `apps/rmcl`，协议 crate 不依赖 GPUI 类型。
- HTTP 与 stdio 使用同一套 `McpServer` 和工具实现。
- 长任务继续由 `TaskManager` 持有，MCP 不建立第二套任务注册表。
- `tasks_snapshots()` 提供进程内任务的只读快照；stdio 与 GUI 不共享内存任务状态。
- 实例、内容、Java 和日志 DTO 与内部领域类型分离，避免内部重构直接破坏 agent 兼容性。

## 传输

### stdio

独立控制台程序 `rmcl-mcp` 使用 rmcp 的 `transport-io`：

```bash
rmcl-mcp stdio --config /path/to/config.toml
```

stdout 只写 MCP 帧，错误和诊断写 stderr。退出时通过 `shutdown_background` 关闭应用内 Tokio runtime，EOF 不会触发 runtime drop panic。独立控制台也避免 Windows GUI 子系统导致 stdout 不可用。

### Streamable HTTP

```bash
rmcl-mcp serve --port 39271
```

监听 `127.0.0.1`，端点 `/mcp`，使用 `StreamableHttpService` 和 loopback host 校验。默认端口为 `39271`，可以显式修改；`0` 被拒绝。当前只实现本地 HTTP，没有远程监听、Bearer token、Origin allowlist 或 GUI 设置开关，后续阶段必须补齐这些安全边界后再考虑远程访问。

## 当前只读工具

| 工具 | 语义 |
| --- | --- |
| `capabilities` | RMCL 版本、平台、路径、传输与工具集合 |
| `instances_list` | 已安装实例及 loader、隔离模式、Java、内存和窗口配置 |
| `instances_get` | 按稳定目录 ID 查询一个实例 |
| `content_list` | 实例的 Mod、资源包和光影及扫描警告，可按 kind 过滤 |
| `java_list` | managed、JAVA_HOME、PATH runtime 与逐候选探测失败 |
| `tasks_list` | 当前与保留的最近任务快照 |
| `tasks_get` | 按 `TaskId` 查询任务快照 |
| `tasks_logs` | 按 cursor 分页读取脱敏日志，单页默认 200、最多 1000 行 |

错误使用 JSON-RPC invalid params 或 internal error；未知实例和未知任务不会被伪装成空结果。结构化响应使用 `structuredContent`，同时保留 rmcp 生成的文本内容。

## 写操作

| 工具 | 语义 |
| --- | --- |
| `instances_create` | 创建 Vanilla 或指定 loader 的实例，返回 `task_id` |
| `instances_update` | 重命名、修改 Java、内存、窗口和隔离模式，返回最新实例快照 |
| `instances_repair` | 检查并修复实例，返回 `task_id` |
| `instances_launch` | 使用当前选中账户启动实例，可选完整修复和服务器目标 |
| `tasks_cancel` | 协作式取消运行中的任务并返回最新快照 |

创建、修复和启动由 `TaskManager` 执行，调用方立即获得 `task_id`，再通过 `tasks_get` 和 `tasks_logs` 观察终态。配置修改是同步短操作，直接返回最新实例数据。Java 覆盖在写入前执行真实 probe；共享模式迁移到隔离模式时复用现有内容复制逻辑。

当前尚未实现 revision、idempotency key、删除实例、远程内容写入、设置页权限分级和跨进程修改锁。HTTP 仍只允许 loopback，不能据此暴露到局域网或公网。

## 后续阶段

1. 本地与远程内容安装、`dry_run`、revision 和 idempotency key。
2. 设置页开关、访问级别和端口。
3. Bearer token、Origin allowlist、跨进程文件锁和 GUI 外部变更刷新。
4. 离线账户、服务器和皮肤接口。
