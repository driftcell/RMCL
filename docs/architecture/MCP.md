# Model Context Protocol（MCP）

状态：实例与内容工作流、稳定 DTO/错误、版本/loader 目录、真实创建预检、实例删除、离线账户、Java ensure、revision、幂等、跨进程写锁和 HTTP 安全已实现；设置页、GUI 外部变更刷新和任务通知尚未实现

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

监听 `127.0.0.1`，端点 `/mcp`，使用 `StreamableHttpService`。默认端口为 `39271`，可以显式修改；`0` 被拒绝。stdio 始终为 `read_write`，HTTP 默认 `read_only`，只有查询和 `instances_create` / `content_install` 的 `dry_run` 可以执行；真实写操作返回 `permission_denied`。

HTTP 还执行以下边界：

- Host 只接受 `127.0.0.1:<port>`、`localhost:<port>` 或 `[::1]:<port>`，其他值返回 400。
- 带 Origin 的请求默认全部返回 403；非浏览器 MCP 客户端没有 Origin 时可以正常访问。可通过重复 `--origin <URL>` 显式放行。
- `--token <TOKEN>` 或 `RMCL_MCP_TOKEN` 配置后，缺少或错误的 Bearer token 返回 401 和 `WWW-Authenticate`。优先使用环境变量，避免 token 出现在进程列表中；token 不写日志，也不参与 Debug 输出。

需要 HTTP 写入时显式提升访问级别：

```bash
RMCL_MCP_TOKEN=<local-secret> rmcl mcp --http --access read_write --origin https://agent.example
```

这些控制仍不构成远程监听能力；当前只绑定 loopback，不能据此暴露到局域网或公网。

## 查询与任务工具

| 工具 | 语义 |
| --- | --- |
| `capabilities` | RMCL 版本、平台、路径、传输与工具集合 |
| `instances_list` | 已安装实例及 loader、隔离模式、Java、内存、窗口配置和 revision |
| `instances_get` | 按稳定目录 ID 查询一个实例 |
| `versions_list` | 官方 Minecraft 目录，支持刷新、ID 子串、类型过滤和数量上限 |
| `loaders_list` | 按 Minecraft 版本查询兼容的 Fabric/Quilt/Forge/NeoForge 版本 |
| `accounts_list` | 离线与 Microsoft 账户元数据，不包含令牌或凭据 |
| `java_list` | managed、JAVA_HOME、PATH runtime 与逐候选探测失败 |
| `java_ensure` | 确保兼容 Java runtime，返回安装/复用任务 |
| `content_list` | 实例的 Mod、资源包和光影及扫描警告，可按 kind 过滤 |
| `content_search` | 按 kind、查询词、游戏版本、loader、排序和分页搜索 Modrinth |
| `content_versions` | 查询 Modrinth 项目的可安装版本、文件和依赖 |
| `tasks_list` | 当前与保留的最近任务快照 |
| `tasks_get` | 按 `TaskId` 查询任务快照 |
| `tasks_logs` | 按 cursor 分页读取脱敏日志，单页默认 200、最多 1000 行 |
| `tasks_cancel` | 协作式取消运行中的任务并返回最新快照 |

错误使用 JSON-RPC invalid params 或 internal error；未知实例和未知任务不会被伪装成空结果。结构化响应使用 `structuredContent`，同时保留 rmcp 生成的文本内容。

## 协议 DTO 与错误

所有工具都在 `tools/list` 中声明 `outputSchema`。只读操作、删除/取消和幂等设置操作还分别声明 `readOnlyHint`、`destructiveHint` 和 `idempotentHint`；带 idempotency key 的可选能力不因此被标记为无条件幂等。

任务快照不再使用 Rust `Debug` 字符串。`kind`、`status` 使用 snake_case；`subject` 以 `type` 区分 version、content 和 instance；`phase` 使用 `kind` 和 `name`；下载、内容、processor 和 native 进度使用带 `type` 的独立结构；游戏退出状态包含 `code`、`signal` 和 `success`。例如：

```json
{
  "kind": "content_install",
  "phase": { "kind": "content", "name": "downloading" },
  "progress": {
    "type": "content",
    "stage": "downloading",
    "current": "mods/example.jar",
    "completed_files": 1,
    "total_files": 2,
    "downloaded_bytes": 50,
    "total_bytes": 100
  }
}
```

旧版调试字符串已由这些 DTO 替代，不维护额外兼容别名；调用方应以当前 `tools/list` 的 schema 为准。

JSON-RPC `error.data` 提供稳定的 `code` 和 `retryable`。调用方错误和冲突使用 invalid params，服务内部错误使用 internal error。当前错误码包括：

| code | 用途与附加字段 |
| --- | --- |
| `invalid_input` | 输入不满足工具约束 |
| `not_found` | 未知实例或任务 |
| `already_exists` | 创建目标已存在，附冲突 `path` |
| `revision_conflict` | revision 不匹配，附 `expected_revision`、`actual_revision` |
| `task_already_running` | 同类任务已运行，附现有 `task_id` |
| `repository_busy` | 另一 RMCL 进程持有仓库写锁，附锁文件 `path`，可稍后重试 |
| `permission_denied` | HTTP `read_only` 模式调用了真实写操作 |
| `conflict` | 安装不完整、incompatible 等状态冲突 |
| `cancelled` | 操作已取消，可按工作流决定是否重试 |
| `unavailable` | 所需应用服务未配置 |
| `internal_error` | 其他内部失败；默认不建议盲目重试 |

## 实例写操作

| 工具 | 语义 |
| --- | --- |
| `instances_create` | 创建 Vanilla 或指定 loader 的实例，返回 `task_id`；支持真实预检和 idempotency key |
| `instances_update` | 重命名、修改 Java、内存、窗口和隔离模式，返回最新实例快照 |
| `instances_repair` | 检查并修复实例，返回 `task_id` |
| `instances_launch` | 使用当前选中账户启动实例，可选完整修复和服务器目标 |
| `instances_delete` | 按 revision 删除实例；`dry_run` 返回删除范围，不修改文件 |

创建、修复和启动由 `TaskManager` 执行，调用方立即获得 `task_id`，再通过 `tasks_get` 和 `tasks_logs` 观察终态。配置修改是同步短操作，直接返回最新实例数据。Java 覆盖在写入前执行真实 probe；共享模式迁移到隔离模式时复用现有内容复制逻辑。

`instances_create` 的 `dry_run` 与实际创建共用预检：校验实例 ID 和目标目录，解析官方基础版本，检查复制源，并按 Minecraft 版本精确匹配 loader 与 installer 版本。响应包含逐项 checks 和非稳定版本 warnings；不会创建目录或任务。`instances_delete` 强制要求匹配的 revision，实际删除只移除 `versions/<id>`，保留共享 libraries 和 assets。

## 账户与 Java

| 工具 | 语义 |
| --- | --- |
| `accounts_select` | 选择已有离线或 Microsoft 账户并保存选择 |
| `accounts_create_offline` | 创建或更新离线账户，可选显式 UUID 和设为当前账户 |
| `accounts_delete` | 删除离线或 Microsoft 账户；Microsoft 先删除系统凭据再更新配置 |

`instances_launch` 额外接受可选 `account { username, kind }`，只影响本次启动，不修改保存的当前账户。Microsoft 登录仍使用现有设备代码流程，MCP 不把 refresh token、access token 或其他凭据放入响应。

`java_ensure` 接受可选 `major_version` 和 Mojang `component`，使用当前平台生成 `JavaRequirement`，返回 `JavaInstall` 任务。任务复用 JavaService 的检测、选择、下载、校验和取消流程，并持有 Java store 自己的 `.rmcl.lock`，直到终态写入后释放。可通过 `tasks_get` 观察阶段，再通过 `java_list` 查看结果。

## 内容写操作

| 工具 | 语义 |
| --- | --- |
| `content_import` | 将本地 Mod、资源包或光影文件复制到实例，支持 `rename` / `reject` 冲突策略 |
| `content_set_enabled` | 启用或禁用本地 Mod 或资源包，返回更新后的条目 |
| `content_delete` | 删除一个本地内容条目；必须提供匹配的 `expected_revision` |
| `content_install` | 安装 Modrinth 版本及 required 依赖，或返回真实预检；支持 idempotency key |

`content_install` 在 `dry_run=true` 时解析版本和 required 依赖图，扫描本地 Mod 元数据，并复用 GUI 的主文件元数据与冲突预检。响应包含主文件、依赖状态、incompatible、optional 数量、警告和应跳过的版本 ID；不会创建内容安装任务或修改实例内容，但可能填充启动器自己的元数据缓存。

实际安装也先执行同一套预检：已满足的依赖通过 `skip_version_ids` 交给现有安装任务，检测到已安装的不兼容 Mod 时拒绝创建任务。安装和下载仍由 `TaskManager` 与 `ContentInstaller` 执行，MCP 不另写下载或依赖解析实现。

## 跨进程写锁

每个 Minecraft 仓库根目录使用一个 `.rmcl.lock` 文件和标准库文件锁。GUI 与 MCP 共享 `rmcl-app` 的锁实现：安装、实例创建、内容安装和游戏任务从注册前持锁到终态写入后，本地内容修改和实例配置修改在整个短操作期间持锁。Java ensure 使用 Java store 根目录下的同名锁，避免并发安装 runtime。不同仓库和 Java store 可以使用不同锁文件并行工作。

锁是非等待式获取；另一个进程已经持锁时返回 `repository_busy`，调用方可以稍后重试。进程内活动任务仍优先返回 `task_already_running`，`register` 在锁内再次检查以保留竞态保护。

带 revision 的操作在同一个 guard 内完成“读取 revision → 校验 → 修改/注册任务”。带 idempotency key 的安装把幂等判定、预检和注册串行化，运行中任务的重复 key 直接重放原 `task_id`，不会再次抢锁或重复预检。

## revision 与幂等

实例快照中的 `revision` 使用 `sha256:` 前缀，内容来自实例版本 JSON 的原始字节；存在 `rmcl.instance.toml` 时再加入分隔字节和 manifest 原始字节。manifest 不存在是合法 legacy 状态，其他读取错误会返回错误。

- `instances_update`、`instances_repair`、`content_import`、`content_set_enabled` 和 `content_install` 接受可选 `expected_revision`。
- `content_delete` 强制要求 `expected_revision`。
- 不匹配时以 invalid params 拒绝，不执行对应修改或创建任务。
- revision 不哈希 Mod、资源包、光影或整个实例目录，因此内容写入后它可以保持不变；它不是内容目录快照，实际互斥由上述仓库写锁提供。

`instances_create` 和 `content_install` 接受可选 `idempotency_key`。进程内注册表按操作、配置文件路径和调用方 key 定位任务；重复调用返回原 `task_id` 和 `idempotent_replay=true`，并发重试在任务注册期间互斥。该注册表不持久化，重启后不会保留，也不能协调多个 RMCL 进程。

## 后续阶段

1. 任务 notifications、设置页开关、端口和 GUI 外部变更刷新。
2. 服务器和皮肤接口。
3. 更完整的内容更新工作流。

HTTP 仍只允许 loopback，不能据此暴露到局域网或公网。
