# Model Context Protocol（MCP）

状态：实例工作流、本地内容写入、Modrinth 搜索/安装、`dry_run`、revision 和进程内 idempotency key 已实现；设置页、权限分级和跨进程修改锁尚未实现

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

同一个 `rmcl` 可执行文件通过子命令使用 rmcp 的 `transport-io`，不需要独立 MCP 程序：

```bash
rmcl mcp --stdio --config /path/to/config.toml
```

stdout 只写 MCP 帧，错误和诊断写 stderr。退出时通过 `shutdown_background` 关闭应用内 Tokio runtime，EOF 不会触发 runtime drop panic。Windows 构建使用控制台子系统以保留 MCP stdio；不带 MCP 子命令的 GUI 启动会调用 `FreeConsole` 释放启动控制台，避免常驻额外控制台窗口。

### Streamable HTTP

```bash
rmcl mcp --http --port 39271
```

监听 `127.0.0.1`，端点 `/mcp`，使用 `StreamableHttpService` 和 loopback host 校验。默认端口为 `39271`，可以显式修改；`0` 被拒绝。当前只实现本地 HTTP，没有远程监听、Bearer token、Origin allowlist 或 GUI 设置开关，后续阶段必须补齐这些安全边界后再考虑远程访问。

## 查询与任务工具

| 工具 | 语义 |
| --- | --- |
| `capabilities` | RMCL 版本、平台、路径、传输与工具集合 |
| `instances_list` | 已安装实例及 loader、隔离模式、Java、内存、窗口配置和 revision |
| `instances_get` | 按稳定目录 ID 查询一个实例 |
| `content_list` | 实例的 Mod、资源包和光影及扫描警告，可按 kind 过滤 |
| `content_search` | 按 kind、查询词、游戏版本、loader、排序和分页搜索 Modrinth |
| `content_versions` | 查询 Modrinth 项目的可安装版本、文件和依赖 |
| `java_list` | managed、JAVA_HOME、PATH runtime 与逐候选探测失败 |
| `tasks_list` | 当前与保留的最近任务快照 |
| `tasks_get` | 按 `TaskId` 查询任务快照 |
| `tasks_logs` | 按 cursor 分页读取脱敏日志，单页默认 200、最多 1000 行 |
| `tasks_cancel` | 协作式取消运行中的任务并返回最新快照 |

错误使用 JSON-RPC invalid params 或 internal error；未知实例和未知任务不会被伪装成空结果。结构化响应使用 `structuredContent`，同时保留 rmcp 生成的文本内容。

## 实例写操作

| 工具 | 语义 |
| --- | --- |
| `instances_create` | 创建 Vanilla 或指定 loader 的实例，返回 `task_id`；支持计划型 `dry_run` 和 idempotency key |
| `instances_update` | 重命名、修改 Java、内存、窗口和隔离模式，返回最新实例快照 |
| `instances_repair` | 检查并修复实例，返回 `task_id` |
| `instances_launch` | 使用当前选中账户启动实例，可选完整修复和服务器目标 |

创建、修复和启动由 `TaskManager` 执行，调用方立即获得 `task_id`，再通过 `tasks_get` 和 `tasks_logs` 观察终态。配置修改是同步短操作，直接返回最新实例数据。Java 覆盖在写入前执行真实 probe；共享模式迁移到隔离模式时复用现有内容复制逻辑。

`instances_create` 的 `dry_run` 只返回创建计划，不创建目录或任务，也不伪装成已经完成远程版本解析。删除实例尚未暴露给 MCP。

## 内容写操作

| 工具 | 语义 |
| --- | --- |
| `content_import` | 将本地 Mod、资源包或光影文件复制到实例，支持 `rename` / `reject` 冲突策略 |
| `content_set_enabled` | 启用或禁用本地 Mod 或资源包，返回更新后的条目 |
| `content_delete` | 删除一个本地内容条目；必须提供匹配的 `expected_revision` |
| `content_install` | 安装 Modrinth 版本及 required 依赖，或返回真实预检；支持 idempotency key |

`content_install` 在 `dry_run=true` 时解析版本和 required 依赖图，扫描本地 Mod 元数据，并复用 GUI 的主文件元数据与冲突预检。响应包含主文件、依赖状态、incompatible、optional 数量、警告和应跳过的版本 ID；不会创建内容安装任务或修改实例内容，但可能填充启动器自己的元数据缓存。

实际安装也先执行同一套预检：已满足的依赖通过 `skip_version_ids` 交给现有安装任务，检测到已安装的不兼容 Mod 时拒绝创建任务。安装和下载仍由 `TaskManager` 与 `ContentInstaller` 执行，MCP 不另写下载或依赖解析实现。

## revision 与幂等

实例快照中的 `revision` 使用 `sha256:` 前缀，内容来自实例版本 JSON 的原始字节；存在 `rmcl.instance.toml` 时再加入分隔字节和 manifest 原始字节。manifest 不存在是合法 legacy 状态，其他读取错误会返回错误。

- `instances_update`、`instances_repair`、`content_import`、`content_set_enabled` 和 `content_install` 接受可选 `expected_revision`。
- `content_delete` 强制要求 `expected_revision`。
- 不匹配时以 invalid params 拒绝，不执行对应修改或创建任务。
- revision 不哈希 Mod、资源包、光影或整个实例目录，因此内容写入后它可以保持不变；它不是内容目录快照，也不替代后续的跨进程修改锁。

`instances_create` 和 `content_install` 接受可选 `idempotency_key`。进程内注册表按操作、配置文件路径和调用方 key 定位任务；重复调用返回原 `task_id` 和 `idempotent_replay=true`，并发重试在任务注册期间互斥。该注册表不持久化，重启后不会保留，也不能协调多个 RMCL 进程。

## 后续阶段

1. 设置页开关、访问级别和端口。
2. Bearer token、Origin allowlist、跨进程文件锁和 GUI 外部变更刷新。
3. 离线账户、服务器和皮肤接口。
4. 实例删除与更完整的内容更新工作流。

HTTP 仍只允许 loopback，不能据此暴露到局域网或公网。
