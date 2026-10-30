# 远程内容与 Modrinth

状态：Modrinth 搜索、资源包/光影画廊浏览、版本列表、正式版推荐、远程项目图标与截图缓存、本地 Mod 元数据索引、安装前置预检、文件下载、SHA-1 校验、必需依赖解析、实例安装和单文件更新已实现；CurseForge、可选依赖自动安装、完整版本约束求解和整合包尚未实现

日期：2026-10-30

## 边界

远程内容分为两层：

- `rmcl-content`：provider-neutral 的远程项目/版本/文件模型和 Modrinth v2 元数据客户端。它只发元数据请求，不下载内容文件。
- `rmcl-app`：`ContentInstaller` 使用现有 `rmcl-net` 下载器获取文件，校验后通过本地内容导入路径发布；`TaskManager` 持有下载、取消、进度和终态。

这样远程下载与游戏文件下载共享并发、重试、超时、SHA-1/size 校验、临时文件和原子发布语义，UI 只消费任务快照。

## 模型

- `RemoteProject`：来源、项目 ID、slug、标题、作者、描述、图标 URL、下载/关注数、分类、游戏版本、加载器、项目类型、更新时间、画廊缩略图和精选画廊图。`preview_url` 优先选择精选图；`gallery_previews` 把精选图放在首位并去重。
- `RemoteVersion`：版本 ID、项目 ID、名称、版本号、更新日志、发布时间、发布类型、游戏版本、加载器、文件、依赖。
- `RemoteFile`：文件名、HTTP(S) URL、可选 size、可选 SHA-1、是否为 primary file。
- `RemoteDependency` / `RemoteDependencyKind`：required、optional、incompatible、embedded。
- `InstalledModIndex`：本地 Mod JAR 的 Mod ID、版本、名称、加载器、依赖和冲突声明；`.disabled` Mod 仍会索引，但标记为未启用。重复 Mod ID 作为警告返回。

`RemoteVersion::installable_file` 优先选择 primary file，回退到第一个符合内容类型扩展名的文件。`recommended_version` 在可安装且带 SHA-1 的版本中按 release > beta > alpha > unknown 选择最新的稳定版本。

## Modrinth 客户端

`ModrinthClient` 使用 v2 API：

- 搜索：`GET /v2/search`
  - facets：`project_type:<mod|resourcepack|shader>`，可选 `versions:<game_version>` 和 `categories:<loader>`。
  - `index`：relevance、downloads、follows、newest、updated。
  - `offset` / `limit` 支持分页；UI 每次加载 20 个。
  - 读取 `gallery` / `featured_gallery` 作为资源包和光影的项目预览；单项目响应同时解析 gallery object 和 `followers` 字段。
- 版本：`GET /v2/project/{id}/version`
  - 使用 `game_versions` 和 `loaders` JSON 数组过滤。
  - `include_changelog=false`；版本按发布时间倒序。
- 单版本：`GET /v2/version/{id}`，用于解析依赖声明的精确版本 ID。
- 单项目：`GET /v2/project/{id}`，用于把依赖的项目 ID 映射到 slug，和本地 Mod ID 做安装前置检查。
- 请求设置明确的 `User-Agent`（`RMCL/<version> (GitHub URL)`）和 30 秒超时。
- 元数据请求支持 `CancellationToken`；HTTP 非 2xx 返回带状态码的 `RemoteContentError::Api`，无效 JSON 不伪装成空结果。

客户端不保存 API key。Modrinth 公共 API 不需要 key；CurseForge 的 key 不应硬编码进 RMCL。

## 依赖版本与元数据

- 本地 Mod 索引和主 Mod 的远程 JAR 元数据会解析 Mod ID、版本、必需依赖和冲突声明。
- 预检在下载前按主文件 SHA-1 缓存远程 JAR 元数据到 `cache/mod-metadata/`；文件缺少 SHA-1 或超过 16 MiB 时跳过远程元数据解析，回退到项目级匹配。
- Fabric/Quilt 的版本谓词使用 `semver` crate；Forge/NeoForge 的 Maven 范围由本地小型解析器处理，版本排序委托 `mvn_version` 的 Maven `ComparableVersion`。
- 已安装前置版本满足主 Mod 声明的范围时，预检标记为已安装并从安装批次跳过；版本不满足时仍安装 Modrinth 解析出的依赖版本并警告。
- 主 Mod 元数据中的 `breaks`/`conflicts` 按声明范围匹配已安装版本，范围外不阻止安装；无范围表示所有版本冲突，缺少版本或无法解析版本/范围时保守保留冲突。
- 依赖图中只有 `required` 会被自动解析；Modrinth 的 optional 只计数，incompatible 做“已安装则阻止”的预检。

## 安装流程

1. UI 从版本列表选择版本；列表标记 `recommended_version` 选出的正式版推荐项。
2. 点击安装先执行本地元数据与前置预检，不立即下载：
   - 解析 required 依赖图；
   - 读取本地 Mod JAR 的 Fabric/Quilt/Forge/NeoForge/legacy Forge 元数据；
   - 按项目 slug/标题规范化后匹配本地 Mod ID；
   - 输出每个前置的“已安装 / 已安装但禁用 / 已安装其他版本 / 将安装 / 冲突”状态。
3. 已安装相同 Mod ID 和相同版本的必需前置从安装批次中跳过，避免重复复制；已安装其他版本时仍安装所需版本并给出警告。
4. Modrinth 的 `incompatible` 依赖对应的 Mod 已安装时阻止安装并列出冲突；optional 只计数，不自动安装。
5. 光影包安装时检测 `iris`、`oculus` 或 `optifine`；缺失时警告“文件可以安装但不会生效”。
6. UI 从版本列表选择可安装文件，要求文件名是单一安全路径组件且 URL 为 HTTP(S)。
7. 缺少 SHA-1 的文件拒绝安装。
8. Mod 安装会先解析 `required` 依赖图：
   - 有 `version_id` 时使用 `GET /v2/version/{id}`；
   - 只有 `project_id` 时使用项目版本列表，按当前 Minecraft 版本和加载器选择第一个可安装版本；
   - optional、incompatible 和 embedded 依赖不自动安装；
   - 依赖先于主文件安装，循环和重复版本被折叠，最多 32 个不同版本。
9. 目标目录中已有同名且 SHA-1 一致的文件（包括 `.disabled` Mod）会跳过，避免重复安装。
10. `TaskManager` 以实例为键注册 `ContentInstall` 任务；同一实例的游戏启动、安装和内容安装互斥。
11. `ContentInstaller` 在临时目录创建目标文件，调用 `DownloadExecutor::download_file` 下载并校验 size/SHA-1。
12. 下载成功后通过 `rmcl-content::import_file` 复制到 `mods/`、`resourcepacks/` 或 `shaderpacks/`。
13. 默认冲突策略为 `Rename`：同名文件变成 `name (1).ext`，不覆盖已有内容。
14. 资源包导入后默认禁用；光影包显示为已安装；Mod 按文件名决定启用状态。
15. 临时目录在导入完成或失败后自动清理。

下载失败、哈希不匹配或取消不会在内容目录留下半文件。依赖安装失败会终止整个批次，已完成的文件保留，未开始的依赖和主文件不会写入。

## 单文件更新

MCP `content_update` 复用相同的远程版本解析、依赖预检和 `ContentInstall` 任务。目标相对路径必须已存在；实际执行在下载与 SHA-1 校验成功后，通过 `replace_content_file` 替换主文件，保留原相对路径、`.disabled` 和资源包启停状态，不使用默认 `name (1).ext` 冲突改名。旧文件先保留为临时备份，替换发布失败时恢复；下载或校验失败不会触碰旧文件。相同版本已经安装时拒绝重复更新。

依赖文件仍按普通安装流程处理，已满足版本通过 `skip_version_ids` 跳过。该语义是单文件替换加依赖导入，不是多文件更新事务，也没有 GUI 批量更新界面。

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
- 项目列表和项目详情页显示远程项目图标；图标下载失败时回退到对应内容类型的通用图标。
- 资源包和光影搜索使用双列作品卡，展示 Modrinth 精选画廊图、作者、简介和下载量；Mod 保持紧凑列表。详情页使用 GPUI Kit `Carousel` 浏览最多 6 张截图，支持按钮、键盘和减少动态效果，并提供推荐版本安装入口与 Modrinth 项目网页链接。
- 截图缓存在配置目录的 `cache/remote-gallery/`；只接受公开 HTTPS 图片地址，源图上限 8 MiB、4096×4096，保留宽高比缩略到最长边 960 的 PNG。图标仍使用独立的 `cache/remote-icons/` 和 256px 缩略规格。图片失败不阻止版本浏览和安装。
- 进入项目后显示可用版本、版本类型、发布时间、游戏版本、加载器、文件名/大小和必需前置数量。
- 正式版推荐项显示“推荐”标签；推荐只在可安装且带 SHA-1 的版本中选择，不保证依赖版本范围完全满足。
- 每个版本提供安装按钮；没有可用文件或缺少 SHA-1 时禁用。点击安装先显示预检对话框，列出主文件、必需前置状态、可选前置数量和警告；存在已安装的不兼容 Mod 时拒绝安装。
- 预检识别本地 Fabric/Quilt/Forge/NeoForge Mod 的 ID、版本、依赖和重复 ID；相同版本前置会从批次中跳过。
- 当前实例没有 Mod 加载器时显示警告：Mod 会下载到实例，但在安装加载器前不会生效。
- 光影包预检检测 Iris/Oculus/OptiFine；缺失时提示文件可以安装但不会生效。已检测到加载器时，版本列表按 `iris`/`optifine` 过滤；未检测到时保留全部版本并显示警告。

## 当前限制

- 只接入 Modrinth，尚未接入 CurseForge。
- 内容可以安装到 Fabric/Quilt/Forge/NeoForge 实例；加载器本身由 [实例创建](LOADERS.md) 安装。
- 只自动安装 required 依赖；optional、incompatible 和 embedded 依赖不自动安装。Modrinth 的 incompatible 只做“已安装则阻止”的预检，不做完整依赖版本约束求解。
- 本地 Mod 元数据支持 Fabric/Quilt/Forge/NeoForge 常见字段和 legacy Forge 的 `mcmod.info` 基础字段；不执行 Mod 代码，也不解析所有可选/嵌套元数据。
- 远程元数据只预检主 Mod 文件（上限 16 MiB）；依赖文件仍按 Modrinth 项目/版本信息判断，不逐个下载依赖 JAR。
- 前置匹配优先使用远程主 Mod 元数据中的真实 Mod ID 和版本范围；元数据缺失时回退到 Modrinth 项目 slug/标题与本地 Mod ID 的规范化比较。
- 前置和本地元数据冲突的版本范围支持常见 Fabric/Quilt semver 谓词和 Forge/NeoForge Maven 区间；不实现完整 SAT、版本回退、可选依赖选择和所有特殊 qualifier。
- 已安装版本只做字符串/范围比较；同一 Mod ID 不同版本会给出警告并按 Modrinth 依赖安装所需版本。
- 依赖去重按版本 ID；已安装识别结合文件名/SHA-1 和本地 Mod ID/版本，不做全局文件哈希索引和更新检查。
- 远程项目图标通过 HTTPS 下载到配置目录的 `cache/remote-icons/`，只接受公开地址和图片内容；缓存按 URL SHA-256 命名，源图尺寸上限 2048×2048，超过 256×256 的图片会缩略后写成 PNG。画廊使用上述独立的 `remote-gallery` 规格，两种缓存都不跟随重定向。
- 不提供完整项目正文渲染；项目网页由系统浏览器打开。
- 不做 API 速率限制的主动排队；429 会作为 API 错误返回，用户可重试。
- 资源包和光影的文件选择依赖远端 primary 文件和扩展名；`rmcl-content` 已提供资源包 `pack_format` 元数据与兼容判断，但在线安装预检和 GPUI 尚未接入该判断。
- 尚未实现整合包、可选依赖自动选择、完整依赖版本约束/冲突求解和更新事务。

## 验证

- `rmcl-content` 使用本地 axum mock 覆盖搜索 facets/index/分页、项目版本解析、单版本/单项目查询、primary file 选择、依赖解析、API 错误和请求取消；元数据测试覆盖 Fabric/Quilt/Forge JAR 解析、依赖/冲突字段、缺失元数据和重复 Mod ID；版本匹配测试覆盖 semver 谓词、Maven 区间和 Forge 版本排序。
- `rmcl-content` 覆盖搜索 gallery/featured_gallery、单项目 gallery/followers 解析、精选图优先和画廊去重；`rmcl-app` 覆盖远程图标 URL 校验、PNG/WebP 解码与缩略、画廊 16:9 宽高比和 960px 缩略、原子缓存、重复 URL 不重复下载和私网地址拒绝。
- `rmcl-app` 集成测试覆盖远程文件下载、SHA-1 校验、同名改名、哈希不匹配不发布、required 依赖图（version ID、project ID、循环、optional 跳过）、依赖优先顺序、重复安装跳过、已安装前置预检和跳过、远程主 JAR 的前置与冲突版本范围判断、元数据缓存、替换后相对路径/启停保持，以及 `TaskManager` 的 `ContentInstall` 快照、subject、批次进度和 Completed 终态。Sodium/Fabric API 冲突范围误报的排查见[调查记录](../../notes/agent/sodium_false_incompatible.md)。
- `rmcl-mcp` 覆盖更新 dry-run 的 `installed_different_version`、真实替换、重复 key 重放、同版本拒绝和依赖去重。
- `apps/rmcl` 已编译并通过现有状态测试；真实窗口中的作品卡、Carousel 截图导航、明暗主题、项目版本切换、安装进度和取消仍需在 macOS、Windows、Linux 逐平台验收。
- 2026-10-25 使用真实 `https://api.modrinth.com/` 做了只读 smoke：带 `project_type:mod`、`versions:1.21.1`、`categories:fabric` facets 的搜索返回 Sodium；Sodium 项目在 `1.21.1` + `fabric` 下的版本列表包含 `primary` 文件、SHA-1/SHA-512 和 size，字段与客户端 DTO 一致。
- 标记为 `#[ignore]` 的真实网络 smoke 安装 Iris 1.8.7（精确依赖 Sodium 版本），确认依赖图返回 Sodium + Iris、依赖先安装、两个文件 SHA-1 校验通过并写入实例 `mods/`。2026-10-25 本机通过。
