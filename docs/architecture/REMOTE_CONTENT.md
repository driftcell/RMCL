# 远程内容与 Modrinth

状态：Modrinth 搜索、版本列表、文件下载、SHA-1 校验、必需依赖解析和实例安装已实现；CurseForge、可选依赖和整合包尚未实现

日期：2026-10-25

## 边界

远程内容分为两层：

- `rmcl-content`：provider-neutral 的远程项目/版本/文件模型和 Modrinth v2 元数据客户端。它只发元数据请求，不下载内容文件。
- `rmcl-app`：`ContentInstaller` 使用现有 `rmcl-net` 下载器获取文件，校验后通过本地内容导入路径发布；`TaskManager` 持有下载、取消、进度和终态。

这样远程下载与游戏文件下载共享并发、重试、超时、SHA-1/size 校验、临时文件和原子发布语义，UI 只消费任务快照。

## 模型

- `RemoteProject`：来源、项目 ID、slug、标题、作者、描述、图标 URL、下载/关注数、分类、游戏版本、加载器、项目类型、更新时间。
- `RemoteVersion`：版本 ID、项目 ID、名称、版本号、更新日志、发布时间、发布类型、游戏版本、加载器、文件、依赖。
- `RemoteFile`：文件名、HTTP(S) URL、可选 size、可选 SHA-1、是否为 primary file。
- `RemoteDependency` / `RemoteDependencyKind`：required、optional、incompatible、embedded。

`RemoteVersion::installable_file` 优先选择 primary file，回退到第一个符合内容类型扩展名的文件。

## Modrinth 客户端

`ModrinthClient` 使用 v2 API：

- 搜索：`GET /v2/search`
  - facets：`project_type:<mod|resourcepack|shader>`，可选 `versions:<game_version>` 和 `categories:<loader>`。
  - `index`：relevance、downloads、follows、newest、updated。
  - `offset` / `limit` 支持分页；UI 每次加载 20 个。
- 版本：`GET /v2/project/{id}/version`
  - 使用 `game_versions` 和 `loaders` JSON 数组过滤。
  - `include_changelog=false`；版本按发布时间倒序。
- 单版本：`GET /v2/version/{id}`，用于解析依赖声明的精确版本 ID。
- 请求设置明确的 `User-Agent`（`RMCL/<version> (GitHub URL)`）和 30 秒超时。
- 元数据请求支持 `CancellationToken`；HTTP 非 2xx 返回带状态码的 `RemoteContentError::Api`，无效 JSON 不伪装成空结果。

客户端不保存 API key。Modrinth 公共 API 不需要 key；CurseForge 的 key 不应硬编码进 RMCL。

## 安装流程

1. UI 从版本列表选择可安装文件，要求文件名是单一安全路径组件且 URL 为 HTTP(S)。
2. 缺少 SHA-1 的文件拒绝安装。
3. Mod 安装会先解析 `required` 依赖图：
   - 有 `version_id` 时使用 `GET /v2/version/{id}`；
   - 只有 `project_id` 时使用项目版本列表，按当前 Minecraft 版本和加载器选择第一个可安装版本；
   - optional、incompatible 和 embedded 依赖不自动安装；
   - 依赖先于主文件安装，循环和重复版本被折叠，最多 32 个不同版本。
4. 目标目录中已有同名且 SHA-1 一致的文件（包括 `.disabled` Mod）会跳过，避免重复安装。
5. `TaskManager` 以实例为键注册 `ContentInstall` 任务；同一实例的游戏启动、安装和内容安装互斥。
6. `ContentInstaller` 在临时目录创建目标文件，调用 `DownloadExecutor::download_file` 下载并校验 size/SHA-1。
7. 下载成功后通过 `rmcl-content::import_file` 复制到 `mods/`、`resourcepacks/` 或 `shaderpacks/`。
8. 默认冲突策略为 `Rename`：同名文件变成 `name (1).ext`，不覆盖已有内容。
9. 资源包导入后默认禁用；光影包显示为已安装；Mod 按文件名决定启用状态。
10. 临时目录在导入完成或失败后自动清理。

下载失败、哈希不匹配或取消不会在内容目录留下半文件。依赖安装失败会终止整个批次，已完成的文件保留，未开始的依赖和主文件不会写入。

## 任务模型

新增：

- `TaskKind::ContentInstall`
- `TaskPhase::Content(ContentInstallPhase)`：Preparing、Downloading、Installing、Completed
- `TaskProgress::Content(ContentDownloadProgress)`：当前文件、批次完成数/总数、阶段、已下载字节、可选总字节和详情
- `TaskSubject::Content { instance_id, kind, name }`

`TaskManager` 提供单文件 `start_content_install` 和带依赖解析的 `start_content_install_with_dependencies`。应用使用 `with_full_and_modrinth` 注入 `ModrinthClient`；测试使用 `with_content_only` 或 `with_content_and_modrinth`。内容任务和其他任务一样支持快照、广播、取消和终态保留。

内容任务完成后，UI 不把实例当成“Minecraft 安装完成”；它只显示内容安装通知，并在实例详情页可见时重新扫描本地内容。

## 界面

实例详情页的“下载内容”按钮进入在线内容页：

- 搜索 Modrinth 项目，按当前实例的 Minecraft 版本过滤；Mod 还会按实例加载器过滤。
- 支持相关度、下载量、关注数、最新发布和最近更新排序，以及“加载更多”。
- 项目列表显示标题、作者、描述、下载量、加载器和分类。
- 进入项目后显示可用版本、版本类型、发布时间、游戏版本、加载器、文件名/大小和必需前置数量。
- 每个版本提供安装按钮；没有可用文件或缺少 SHA-1 时禁用。安装 Mod 时会自动解析并安装 required 依赖。
- 当前实例没有 Mod 加载器时显示警告：Mod 会下载到实例，但在安装加载器前不会生效。
- 版本行显示必需前置数量；optional 和 incompatible 依赖仍只展示数量，不自动处理。

## 当前限制

- 只接入 Modrinth，尚未接入 CurseForge。
- 内容可以安装到 Fabric/Quilt/Forge/NeoForge 实例；加载器本身由 [实例创建](LOADERS.md) 安装。
- 只自动安装 required 依赖；optional、incompatible 和 embedded 依赖不处理，也不做依赖版本冲突求解。
- 不解析 Mod 元数据来显示真实 Mod 名称、图标或版本兼容性；项目/版本信息来自远端。
- 依赖去重只按版本 ID；已安装识别按文件名和 SHA-1，不做全局文件哈希索引和更新检查。
- 不显示远程项目图标，也不提供项目网页跳转。
- 不做 API 速率限制的主动排队；429 会作为 API 错误返回，用户可重试。
- 资源包和光影的文件选择依赖远端 primary 文件和扩展名，不做 `pack_format` 兼容判断。
- 尚未实现整合包、optional/incompatible 依赖处理、依赖冲突求解和更新事务。

## 验证

- `rmcl-content` 使用本地 axum mock 覆盖搜索 facets/index/分页、项目版本解析、单版本查询、primary file 选择、依赖解析、API 错误和请求取消。
- `rmcl-app` 集成测试覆盖远程文件下载、SHA-1 校验、同名改名、哈希不匹配不发布、required 依赖图（version ID、project ID、循环、optional 跳过）、依赖优先顺序、重复安装跳过，以及 `TaskManager` 的 `ContentInstall` 快照、subject、批次进度和 Completed 终态。
- `apps/rmcl` 已编译并通过现有状态测试；真实窗口中的搜索、项目版本切换、安装进度和取消仍需在 macOS、Windows、Linux 逐平台验收。
- 2026-10-25 使用真实 `https://api.modrinth.com/` 做了只读 smoke：带 `project_type:mod`、`versions:1.21.1`、`categories:fabric` facets 的搜索返回 Sodium；Sodium 项目在 `1.21.1` + `fabric` 下的版本列表包含 `primary` 文件、SHA-1/SHA-512 和 size，字段与客户端 DTO 一致。
- 标记为 `#[ignore]` 的真实网络 smoke 安装 Iris 1.8.7（精确依赖 Sodium 版本），确认依赖图返回 Sodium + Iris、依赖先安装、两个文件 SHA-1 校验通过并写入实例 `mods/`。2026-10-25 本机通过。
