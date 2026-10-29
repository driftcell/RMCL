# Model Context Protocol（MCP）

状态：实例与内容工作流、稳定 DTO/错误、版本/loader 目录、真实创建预检、实例删除、离线账户、Java ensure、服务器收藏与状态、本地皮肤库与正版皮肤工作流、任务通知、revision、幂等、跨进程写锁、HTTP 安全、CLI 传输、内置 Agent 进程内连接和外部变更刷新已实现

日期：2026-10-29

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

stdout 只写 MCP 帧，错误和诊断写 stderr。退出时通过 `shutdown_background` 关闭应用内 Tokio runtime，EOF 不会触发 runtime drop panic。Windows 构建使用控制台子系统以保留 MCP stdio；不带 MCP 子命令的 GUI 启动会调用 `FreeConsole` 释放启动控制台，避免常驻额外控制台窗口。GUI 启动的 Java 子进程通过 `CREATE_NO_WINDOW` 禁止 Windows 为它们新建控制台窗口；见 [启动计划与游戏进程](LAUNCH.md#进程与日志)。

### 客户端接入

支持 `mcpServers` 配置的客户端可以直接启动同一个 `rmcl` 可执行文件，不需要 `rmcl-mcp` 或额外 shim：

```json
{
  "mcpServers": {
    "rmcl": {
      "command": "/absolute/path/to/rmcl",
      "args": ["mcp", "--stdio"]
    }
  }
}
```

省略 `--config` 时使用[平台默认配置路径](CONFIGURATION.md#路径)。需要隔离测试配置时，将绝对路径加入 `args`：`["mcp", "--stdio", "--config", "/absolute/path/to/config.toml"]`。不同客户端的顶层设置键可能不同，但 `command` 和 `args` 应保持上述直接进程形式，客户端还需支持 MCP stdio 的换行分隔 JSON-RPC 帧。

Windows 示例（路径按实际安装位置和用户目录修改）：

```json
{
  "mcpServers": {
    "rmcl": {
      "command": "C:\\Tools\\RMCL\\rmcl.exe",
      "args": [
        "mcp",
        "--stdio",
        "--config",
        "C:\\Users\\example\\AppData\\Roaming\\rmcl\\config.toml"
      ]
    }
  }
}
```

Windows 客户端应直接创建 `rmcl.exe` 子进程并继承 stdin/stdout 管道，不要通过 `cmd /c start`、快捷方式或其他会分离进程的方式启动。MCP 子命令不会进入 GPUI 初始化，也不会调用 `FreeConsole`；关闭 stdin 后进程应正常退出。`apps/rmcl/tests/mcp_stdio.rs` 在 Windows、Linux 和 macOS CI 中启动真实二进制，覆盖握手、工具元数据、订阅/退订和 EOF。GUI 启动后没有常驻控制台则仍需使用 Windows 发布包手工验收，stdio smoke 不替代该项图形检查。

### Streamable HTTP

```bash
rmcl mcp --http --port 39271
```

监听 `127.0.0.1`，端点 `/mcp`，使用 `StreamableHttpService`。默认端口为 `39271`，可以显式修改；`0` 被拒绝。stdio 始终为 `read_write`，HTTP 默认 `read_only`，查询、任务订阅和 `instances_create` / `content_install` 的 `dry_run` 可以执行；真实写操作返回 `permission_denied`。

HTTP 还执行以下边界：

- Host 只接受 `127.0.0.1:<port>`、`localhost:<port>` 或 `[::1]:<port>`，其他值返回 400。
- 带 Origin 的请求默认全部返回 403；非浏览器 MCP 客户端没有 Origin 时可以正常访问。可通过重复 `--origin <URL>` 显式放行。
- `--token <TOKEN>` 或 `RMCL_MCP_TOKEN` 配置后，缺少或错误的 Bearer token 返回 401 和 `WWW-Authenticate`。优先使用环境变量，避免 token 出现在进程列表中；token 不写日志，也不参与 Debug 输出。

需要 HTTP 写入时显式提升访问级别：

```bash
RMCL_MCP_TOKEN=<local-secret> rmcl mcp --http --access read_write --origin https://agent.example
```

这些控制仍不构成远程监听能力；当前只绑定 loopback，不能据此暴露到局域网或公网。

### GUI 与内置 Agent

GUI 不宿主供外部客户端使用的 MCP 服务，设置页也不提供 MCP 配置。外部客户端统一使用上述 CLI stdio 或 Streamable HTTP 传输。

内置 Agent 通过独立的进程内 `rmcp` duplex transport 连接同一个 `AppContext` 和 `TaskManager`，不启动子进程，也不读取 CLI MCP 配置。详见[内置 AI 助手](AGENT.md#mcp-连接)。

### GUI 外部变更刷新

窗口重新激活时，GUI 会重新扫描实例，并重新读取配置中的账户、服务器收藏和选中版本；打开实例详情且没有内容操作进行中时也会重新扫描本地内容。这覆盖外部 MCP 进程造成的文件和配置变化，不需要常驻文件监听或定时轮询。

刷新只合并当前 MCP 能修改的字段，不替换账户编辑文本、外观或其他本地设置，也不会把刷新后的内存状态自动保存回文件。配置读取失败时保留当前内存数据并显示警告。外部 CLI 进程不共享内存任务状态；内置 Agent 通过独立的进程内连接共享上述 TaskManager。

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
| `servers_list` | 服务器收藏元数据，不发起网络请求 |
| `servers_ping` | 按收藏 ID 或临时地址执行 Java Edition SLP，返回 MOTD、人数、延迟和图标缓存路径 |
| `skins_list` | 本地皮肤库元数据、模型提示和头像缓存路径 |
| `skins_profile` | 刷新正版资料，返回 owned skins/capes，不包含令牌 |
| `content_list` | 实例的 Mod、资源包和光影及扫描警告，可按 kind 过滤 |
| `content_search` | 按 kind、查询词、游戏版本、loader、排序和分页搜索 Modrinth |
| `content_versions` | 查询 Modrinth 项目的可安装版本、文件和依赖 |
| `tasks_list` | 当前与保留的最近任务快照 |
| `tasks_get` | 按 `TaskId` 查询任务快照 |
| `tasks_wait` | 等待任务进入终态；游戏启动任务在进入 `RunningGame` 时返回。默认 120 秒，最长 600 秒，超时返回当前快照并标记 `timed_out`；失败时附最后 20 行脱敏日志 |
| `tasks_logs` | 按 cursor 分页读取脱敏日志，单页默认 200、最多 1000 行 |
| `tasks_subscribe` | 当前 MCP session 显式订阅任务事件，可按 task 过滤并选择是否包含日志 |
| `tasks_unsubscribe` | 取消当前 session 的一个订阅并等待转发器停止 |
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

## 任务通知

RMCL 默认不向 MCP 客户端推送任务事件。客户端必须调用 `tasks_subscribe`：省略 `task_id` 时订阅全部任务，提供时只转发该任务；`include_logs` 默认 `false`。响应包含 `subscription_id` 和建立订阅时的 `items` 快照。广播接收器先于快照创建，因此订阅建立期间发生的事件会保留在有界队列中，可能与初始快照重复，客户端应按 task ID 和最新状态处理。

`tasks_unsubscribe` 取消并等待对应转发器退出。连接或 session 结束时，其所有订阅也会取消；订阅 ID 只在创建它的 MCP session 内有效。HTTP `read_only` 允许订阅和退订，因为这不修改启动器数据。

通知使用自定义方法 `notifications/rmcl/task`，`params` 总是包含 `subscription_id` 和 `type`：

| type | 附加字段与语义 |
| --- | --- |
| `snapshot` | `task` 为与 `tasks_get` 相同的稳定快照 DTO |
| `log` | `task_id`、单调行号 `line` 和脱敏 `log { stream, text }`；仅 `include_logs=true` 时发送 |
| `dismissed` | `task_id` 对应的保留任务已被移除 |
| `lagged` | `missed` 表示有界广播队列丢弃的事件数，客户端应重新同步 |

例如：

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/rmcl/task",
  "params": {
    "subscription_id": 1,
    "type": "snapshot",
    "task": {
      "id": 7,
      "kind": "java_install",
      "subject": { "type": "java", "major_version": 21, "component": null },
      "status": "completed",
      "phase": { "kind": "installing_java" },
      "progress": null,
      "error": null,
      "exit": null,
      "stop_requested": false,
      "log_lines": 0
    }
  }
}
```

TaskManager 的广播和 rmcp 的 peer 队列均有界。慢客户端可能丢失高频进度或日志，但不会造成无界内存排队，也不会阻塞任务执行。收到 `lagged` 后，使用 `tasks_get` 获取最新快照，并使用 `tasks_logs` 的 cursor 补齐日志；通知不是持久、无丢失的事件日志。日志订阅只转发订阅后的新日志，历史日志仍通过 `tasks_logs` 获取。

stdio 直接接收这些 JSON-RPC notification 帧。使用 Streamable HTTP 的客户端还需按 MCP 传输约定接收服务端 SSE 事件；仅读取各个 POST 的 JSON 响应不能替代异步通知流。

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

## 服务器收藏与状态

| 工具 | 语义 |
| --- | --- |
| `servers_upsert` | 创建或按 ID 更新服务器收藏，使用与 GUI 相同的地址校验和默认显示名 |
| `servers_delete` | 按 ID 删除收藏，不读取或修改游戏内 `servers.dat` |
| `servers_ping` | 按收藏 ID 或临时地址执行状态查询，不保存临时地址 |

`servers_list` 只返回 `id`、`name` 和 `address`，不进行 DNS 或网络访问。`servers_ping` 要求 `server_id` 与 `address` 恰好提供一个，复用现有 SLP 的超时、响应上限、MOTD 扁平化、人数/版本解析和 favicon 校验；图标按内容 SHA-1 写入现有缓存，响应只返回本地路径。MOTD、人数、延迟、版本和图标路径都是瞬态数据，不写入配置。

无 ID 的 `servers_upsert` 每次都会创建新 UUID，因此该工具不声明无条件 `idempotentHint`；带 ID 更新使用同一书签。GUI 与 MCP 共用 `upsert_server_bookmark` / `remove_server_bookmark`，并在窗口重新激活时刷新外部收藏变化。

## 本地皮肤库

| 工具 | 语义 |
| --- | --- |
| `skins_import` | 校验本地 PNG 或 HTTPS 图片直链并导入皮肤库，可选显示名 |
| `skins_rename` | 修改显示名，保持内容 ID 不变 |
| `skins_delete` | 删除库条目和完整纹理，不影响账户当前皮肤 |

导入复用现有大小、64×64/64×32、PNG 解码和官方上传字节规范化。HTTPS 来源拒绝 URL 凭据、私网/回环地址、非 PNG 内容和超限重定向。文件准备完成后，在皮肤库自己的 `.rmcl.lock` 内更新索引，与库内 mutex 共同协调跨进程写入。重复内容使用同一个 SHA-256 ID，不增加重复条目；`source_url`、模型提示和头像路径只作为本地元数据返回，不暴露图片字节或账户凭据。

这些工具只管理本地库，不调用 Microsoft/Mojang 换肤接口，也不改变当前激活皮肤。

## 正版皮肤工作流

| 工具 | 语义 |
| --- | --- |
| `skins_profile` | 使用缓存凭据刷新资料，返回玩家名、UUID、当前皮肤和 owned skins/capes |
| `skins_apply` | 上传本地库 ID 或文件/HTTPS 来源的 PNG，指定 `classic` 或 `slim`，成功后写入本地库 |
| `skins_reset` | 恢复默认皮肤；服务不返回资料时立即刷新 |

三个工具都复用 `MicrosoftAccountService` 的凭据加载、刷新、上传和 reset，不在响应中包含 refresh token 或 access token。`username` 省略时使用当前选中的 Microsoft 账户；离线选择或未知账户会拒绝。apply 要求 `skin_id` 与 `source` 恰好提供一个，模型必须是 `classic` 或 `slim`，并保留现有图片和 URL 安全校验。真实上传和 reset 需要 `read_write`，HTTP 默认只读模式返回 `permission_denied`。

上传成功后才会写入本地皮肤库。若远程上传已成功但本地写入失败，返回 `conflict` 并说明部分成功，调用方应先通过 `skins_profile` 和 `skins_list` 刷新，不应盲目重试。`skins_reset` 对状态具有幂等性，但每次仍会调用官方 reset；owned skins 的按 ID 一键切换仍未暴露，apply 是下载校验后的 multipart 上传。

## 内容写操作

| 工具 | 语义 |
| --- | --- |
| `content_import` | 将本地 Mod、资源包或光影文件复制到实例，支持 `rename` / `reject` 冲突策略 |
| `content_set_enabled` | 启用或禁用本地 Mod 或资源包，返回更新后的条目 |
| `content_delete` | 删除一个本地内容条目；必须提供匹配的 `expected_revision` |
| `content_install` | 安装 Modrinth 版本及 required 依赖，或返回真实预检；支持 idempotency key |
| `content_update` | 用指定 Modrinth 版本替换现有内容，保持相对路径和启停状态；支持预检与 idempotency key |

`content_install` 在 `dry_run=true` 时解析版本和 required 依赖图，扫描本地 Mod 元数据，并复用 GUI 的主文件元数据与冲突预检。响应包含主文件、依赖状态、incompatible、optional 数量、警告和应跳过的版本 ID；不会创建内容安装任务或修改实例内容，但可能填充启动器自己的元数据缓存。

实际安装也先执行同一套预检：已满足的依赖通过 `skip_version_ids` 交给现有安装任务，检测到已安装的不兼容 Mod 时拒绝创建任务。安装和下载仍由 `TaskManager` 与 `ContentInstaller` 执行，MCP 不另写下载或依赖解析实现。

`content_update` 额外要求目标 `relative_path` 已存在，并使用同一 revision 检查和仓库锁。`dry_run` 返回目标路径及完整依赖预检；实际执行只在版本和 SHA-1 验证成功后替换主文件，保留旧相对路径与 `.disabled`/资源包启停状态，不留下第二个版本。替换发布失败会恢复旧文件；下载或校验失败不会触碰旧文件。相同版本已经安装时返回 `conflict`，依赖仍按普通安装流程处理。这是单文件替换加依赖导入，不是完整的多文件更新事务。

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

1. CurseForge、整合包和更完整的依赖求解。

HTTP 仍只允许 loopback，不能据此暴露到局域网或公网。
