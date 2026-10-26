# 桌面界面与 GPUI 桥接

状态：启动器壳、实例创建与隔离迁移、Fabric/Quilt/Forge/NeoForge 加载器安装、本地与 Modrinth 远程 Mod/资源包/光影管理、服务器收藏与一键进服、下载、设置、离线/正版账户、皮肤头像和任务日志界面已实现

日期：2026-10-26

## GPUI Kit 基线

桌面应用使用 `gpui-kit = 0.8.0`，由其统一提供匹配的 `gpui-pre = 0.3.9`、Base、Component 和默认 assets。应用只直接依赖 `gpui-kit` 并导入 `gpui_kit::*`，不单独覆盖 GPUI 包或混用其他快照。

应用启用默认 `component + assets`，并使用 `gpui_kit::assets::AllAssets` 获取完整 Lucide 图标目录（约多 1 MiB）。没有启用 `gpui-fast`、speech、tree-sitter、inspector 或 profiler，也没有 mold/LTO 等二进制优化。

平台依据：[GPUI Kit Installation](https://gpui-kit.com/docs/installation)。macOS 需要 15+ 和 Xcode Command Line Tools；Windows 需要 Windows 10+、MSVC C++ 工具链、Windows SDK 和 CMake；Ubuntu 24.04 的 CI 安装文档列出的 fontconfig、Wayland、WebKitGTK、XKB/X11、SSL、zstd 和 Vulkan 开发库。Linux 显示窗口还需要图形会话和可用 Vulkan 驱动，headless CI 编译不能代替运行验证。

调研记录：[GPUI Kit 技术验证](../../notes/development/gpui_kit_bootstrap.md)、[界面重做与配置记录](../../notes/development/ui_redesign.md)。

## 窗口与导航

窗口使用 GPUI Kit 的 `TitleBar::window_options()` 自定义标题栏，默认尺寸 1180×780，最小 980×640。macOS 保留原生红绿灯；Windows 和 Linux 使用 TitleBar 自带的窗口控制。标题栏左侧只保留 RMCL 品牌标记（简化 R）、名称和版本，右侧是任务阶段与明暗主题切换。

主界面是常驻侧栏 + 内容区的桌面壳：

- 侧栏宽 232px，包含“主页”“实例”“服务器”“下载游戏”“日志”和“设置”。日志放在单独的“工具”分组，不占用主页空间；任务运行中显示 spinner，任务失败或游戏异常退出后显示红点。
- 侧栏底部是当前账户卡片和编辑按钮，账户状态在所有页面可见；正版账户显示官方皮肤头像，离线账户显示玩家名首字母。
- 内容区底部是常驻任务栏，显示任务阶段、当前文件、进度、停止按钮和平台信息。切换页面不会丢失任务状态。主页不重复展示进度条，底栏是唯一常驻进度位置。
- 页面之间使用即时切换，不做 HMCL 式转场动画。颜色、间距和圆角优先来自 GPUI Kit 语义 token；侧栏宽度、图标尺寸、虚拟列表行高等物理边界使用明确的像素值。

## 页面

### 主页

- 大号启动卡片：实例图标、显示名称、类型和发布时间、实际游戏目录，以及“隔离/共享”标签。
- “启动配置”面板：可搜索的已安装实例下拉框、可搜索的账户下拉框、主启动按钮。
- 没有实例时显示“安装第一个实例”入口；没有账户时显示添加账户入口，并禁用启动。
- “最近实例”卡片列出最多三个已安装实例，可直接启动。
- “快速操作”卡片提供安装新版本和打开游戏目录。
- 主页不重复展示下载进度；进度、当前文件和停止入口只在底部任务栏。

### 实例

- 扫描 `versions/<id>/<id>.json`，读取可选的 `rmcl.instance.toml`，显示有效实例；按发布时间从新到旧排序。
- 搜索框按实例 ID、显示名称和基础 Minecraft 版本过滤，并提供“全部 / 正式版 / 快照 / 旧版”筛选。
- 每行显示显示名称、基础版本、加载器（若有）、隔离模式和实际游戏目录。manifest 损坏时标记“配置异常”，但仍允许修复或删除。
- 顶部“新建实例”进入创建页；每行提供启动、管理内容、检查并修复、启用版本隔离、打开游戏目录和删除。检查修复复用 `VersionSource::Existing`，不新增第二套安装逻辑。
- 有共享实例时页面顶部显示迁移提示；没有 manifest 的旧版本显示“共享”标签。点击启用隔离后弹出确认，确认后把全局 mods、资源包和光影中缺失的文件复制到实例，不覆盖已有文件，原全局内容保留。
- 正常启动走已安装快速路径：读取版本 JSON、生成计划并检查 client、libraries、natives 和 asset index 是否存在，不重新下载或重新哈希 asset objects。缺少基础文件时返回“版本不完整，需要修复”，不会静默启动。
- 删除只移除 `versions/<id>/`，保留共享 `libraries/` 和 `assets/`；隔离实例会同时删除自己的 mods、资源包、光影和存档，确认文案会明确说明。删除前要求版本 JSON 和 ID 匹配，并拒绝符号链接目录。
- 缺少客户端 JAR 的实例标记为“文件不完整”，仍可启动或修复；修复流程会补齐缺失文件。

### 新建实例

- 输入 Minecraft 版本 ID，选择 Vanilla、Fabric、Quilt、Forge 或 NeoForge。
- 加载器通过官方元数据获取版本列表，稳定版默认优先；Forge 1.12.2 及更早的 legacy installer 不展示为可安装选项。
- 显示名称和实例 ID 可留空；ID 按基础版本和加载器自动生成，也可手动指定。
- 创建过程显示基础版本安装、加载器元数据获取、实例配置写入、Forge/NeoForge processor 进度和实例文件安装阶段。
- 创建成功后自动选中新实例并进入实例详情。

### 服务器

- 侧栏“服务器”页展示全局收藏，使用当前选中实例和账户进服；服务器条目不绑定实例或账户。
- 顶部显示当前启动目标、手动刷新全部和添加服务器；页面可见时每 60 秒自动刷新状态。
- 每行显示服务器图标、名称、地址、MOTD、在线/上限人数、版本、连接延迟和在线/离线状态；支持搜索、单行刷新、编辑和删除。
- 添加/编辑对话框校验 `host[:port]`；删除只移除 RMCL 收藏，不修改游戏内 `servers.dat`。
- “进入”通过 `--quickPlayMultiplayer`（1.20 / 23w14a+）或 legacy `--server`/`--port` 启动当前实例，仍复用底部任务栏、日志和取消。
- 没有实例、账户或已有任务时禁用进服并显示原因；状态查询失败在行内展示，不弹全局错误。

### 实例详情

- 从实例行的“管理内容”按钮进入；顶部显示实例名称、隔离状态、加载器、基础版本、启动和打开实例目录，并提供重命名按钮。重命名只修改显示名称，不改目录。
- 实例摘要显示游戏目录、内容数量、Java 和启动设置；“选择 Java”列出本机探测到的 managed、`JAVA_HOME` 和 PATH runtime，也可输入自定义 `java`/`java.exe` 路径并先探测，可一键改回自动选择。显式 Java 只影响当前实例，启动前重新校验。
- “启动设置”可设置最大内存（MiB）和初始窗口宽高；留空保留 JVM/游戏默认。输入在保存时校验范围。
- 三个标签页分别管理 Mods、资源包和光影包，标签显示数量；页面显示游戏目录和每类已启用/总数。
- 搜索按显示名称和相对路径过滤；工具栏提供导入、打开内容目录和重新扫描。
- 导入使用原生文件选择器；同名文件自动改名为 `name (1).ext`，不覆盖已有文件。
- Mod 和资源包提供启用/禁用按钮；Mod 使用 `.disabled` 后缀，资源包改写实例 `options.txt`；光影包只管理文件。
- 每行显示大小或“文件夹”、相对路径和状态；删除前弹出确认。扫描警告和 `options.txt` 错误显示为横幅，不隐藏其他内容。
- 内容操作进行中时禁用导入、切换、删除和启动，避免与游戏进程或后续扫描冲突。
- “下载内容”进入在线内容页，按当前实例的 Minecraft 版本过滤；Mod 还会按实例加载器过滤。

### 在线内容

- 搜索 Modrinth 项目，支持相关度、下载量、关注数、最新发布和最近更新排序，以及分页“加载更多”。
- 项目列表显示标题、作者、描述、下载量、加载器和分类；进入项目后显示可用版本、版本类型、发布时间、游戏版本、加载器、文件名/大小和必需前置数量。
- 每个版本可直接安装；下载、SHA-1 校验和写入实例由 `ContentInstall` 后台任务执行，进度和取消复用底部任务栏。
- 当前实例没有 Mod 加载器时显示警告；安装 Mod 时会自动解析并安装 required 依赖，optional/incompatible 依赖只显示数量。
- 安装完成后刷新实例本地内容；同名文件自动改名，不覆盖。

### 下载游戏

- 首次打开时拉取并缓存 Mojang `version_manifest_v2.json`，显示版本总数和最后获取状态。
- 搜索框按版本 ID 过滤，默认显示正式版；可切换快照和旧版，不提供“全部”标签。
- 使用 `v_virtual_list` 渲染远程版本，避免一次绘制数百行。
- 每行显示类型图标、版本 ID、类型标签、发布时间、已安装状态和安装 / 重新安装按钮。已安装状态使用绿色圆形对勾，不再占用一整枚文字标签。
- 安装按钮打开确认对话框，说明新实例默认启用版本隔离，确认后启动应用层安装任务；进度只显示在底部任务栏，不弹出阻塞式进度窗口。
- 同时只允许一个安装、修复、启动或删除操作。任务运行期间，安装、启动、修复和删除按钮都禁用，避免并发写入同一仓库。

### 日志

- 侧栏独立入口；任务运行中显示 spinner，异常后显示红点。
- 显示任务类型、状态、阶段、进度、错误、退出码或终止信号。
- 日志使用等宽字体，支持自动滚动、复制、导出到配置目录下的 `logs/`、打开日志目录和清除当前任务记录。
- 用户手动滚动日志时自动滚动立即关闭，直到重新打开；游戏正常启动和正常退出不弹 toast，异常退出才提示并标记日志入口。
- 取消或异常退出后日志仍保留，直到用户清除或新任务开始。

### 设置

按卡片分组，不采用 HMCL 的八标签设置树：

- **游戏**：Minecraft 目录使用原生目录选择器，Java 目录、自动获取 Java。目录修改后立即重新扫描版本，不移动现有文件；Java 目录和自动获取设置重启后生效。
- **启动行为**：启动器可见性。默认保持显示；选择“游戏运行时隐藏”后，游戏进入 `RunningGame` 阶段时最小化启动器，任务退出、失败或取消时恢复。
- **外观**：跟随系统 / 浅色 / 深色，以及翡翠、海蓝、紫罗兰、琥珀、玫瑰五种强调色。修改立即应用并保存。
- **关于**：版本、配置路径、运行平台和 GPUI Kit 文档链接。

## 账户

账户模型统一展示离线与 Microsoft 正版账户：

- 离线账户由 `rmcl-auth` 生成稳定 UUID；用户名允许非传统字符，界面按原始输入保留。
- 正版账户使用 Microsoft 设备代码登录，成功后保存 profile UUID、玩家名、官方皮肤 URL 和系统钥匙串凭据。
- 主页账户下拉只显示玩家名，不附加“正版/离线”标记；内部用稳定账户键区分同名账户。
- 侧栏卡片和账户列表显示正版皮肤头像；无皮肤或离线账户显示首字母。
- 账户对话框支持切换、添加离线账户、正版登录和删除。删除前弹出确认；删除正版账户会同时清除系统凭据，删除当前账户后自动切换到下一个账户。
- 启动正版账户时按需刷新 token；刷新失败提示重新登录，不会回退成离线身份。

协议、令牌和皮肤细节见 [正版账号、令牌与皮肤](AUTHENTICATION.md)。当前不包含 authlib-injector、第三方皮肤站、披风和皮肤上传。

## 配置与路径

启动器配置由 `rmcl-store` 持久化，默认路径：

- macOS / Linux：`~/.config/rmcl/config.toml`，`XDG_CONFIG_HOME` 存在时优先使用它。
- Windows：`%APPDATA%\rmcl\config.toml`。

配置使用 TOML、schema 版本 2（可读取 schema 1 并升级），并通过同目录临时文件、`sync` 和原子替换写入。损坏或更高版本的配置不会被静默覆盖；界面显示读取错误，设置修改仍会写入当前 schema。详细字段和错误语义见 [启动器配置](CONFIGURATION.md)。

Java store 仍使用 `~/.rmcl/java`，日志位于配置目录下的 `logs/`。RMCL 不读取或改写 HMCL 配置。

## 事件桥接与状态一致性

`apps/rmcl` 在窗口根持有 `Rc<Services>`：

- 一个 multi-thread Tokio `Runtime`，驱动 `rmcl-app` 的安装、Java 和游戏任务。
- 一个 `TaskManager`，持有全部后台任务、取消令牌、快照和日志。
- 一个 `VersionCatalog`，缓存远程版本清单。
- 一个 `MicrosoftAccountService`，负责设备代码登录、token 刷新、系统凭据读写和启动身份解析。
- 一个 `SkinCache`，只从 Mojang 官方纹理域下载并合成头像。
- 一个 `ConfigStore`，负责 TOML 配置。
- 当前目标 `Platform`，供每个请求克隆使用。

GPUI 不使用 `block_on` 等待核心任务。按钮回调通过 `Runtime::spawn` 调用 `start_install`、`start_launch` 或 `cancel`，并在 GPUI foreground task 中 await 返回的 `JoinHandle`。传给 Tokio 的数据必须是 owned `Send` 数据，不捕获 GPUI Context 或 Window。

`TaskManager` 的广播接收器直接移入一个持久保存的 GPUI foreground task。该 task 只处理事件和短 Entity 更新，不拥有安装或游戏 future。后台任务不依赖 View 是否正在显示日志。

版本选择使用 `SelectState<SearchableVec<String>>`；账户选择使用带稳定账户键的 `AccountSelectItem`，显示名不含类型后缀，同名离线与正版账户仍可区分。安装完成或目录切换后通过 `set_items` 更新，不重建整个 View。版本请求带稳定 `TaskId`，终态只触发一次续期、版本重扫和通知。

实例详情页的本地内容扫描、导入、切换和删除通过 `Runtime::spawn` + `spawn_blocking` 执行；界面持有短生命周期 foreground task，只负责等待结果和更新 `ContentLibrary`。这些本地操作不进入 `TaskManager`，因为它们没有长时下载或游戏进程生命周期。

Modrinth 搜索和版本列表也通过短生命周期 foreground task 调用元数据客户端，并用请求 generation/cancellation 防止旧结果覆盖新页面。远程文件安装进入 `TaskManager` 的 `ContentInstall` 任务，复用状态栏进度、取消、广播快照和实例互斥。

## 明暗主题与系统外观

`gpui_kit::init` 提供默认浅色和深色主题。应用在启动和设置变化时调用：

- 跟随系统：`Theme::sync_system_appearance`，并让原生窗口外观跟随系统。
- 浅色 / 深色：`Theme::change(ThemeMode::…)`，同时设置 macOS 原生窗口外观。
- 强调色：在主题切换后通过 `Theme::update` 覆盖 `primary`、`button_primary*`、`sidebar_primary`、`sidebar_accent`、`ring` 和 `selection`。

标题栏的主题按钮在浅色和深色之间切换；选择“跟随系统”时，窗口外观变化会重新应用强调色。主题和强调色修改立即保存。

## 当前不包含

authlib-injector、OptiFine 等加载器、legacy Forge installer、给已有实例追加/升级/移除加载器、实例级任意启动参数、实例复制与导入导出、CurseForge、内容更新检查、整合包、Java runtime 列表与自动下载管理界面、披风与皮肤上传、Unicode 完整本地化、崩溃原因分析、自动更新和安装包。

## 验证

- `apps/rmcl/src/state.rs` 的纯 Rust 测试覆盖任务切换、日志隔离、重复/过期输入、有界缓冲和终态通知去重。
- `rmcl-store` 覆盖 TOML 往返、schema 1 升级、schema 拒绝、损坏文件保留、原子覆盖和默认路径。
- `rmcl-app` 覆盖版本扫描、远程 manifest 缓存、取消、快速启动、缺失文件、版本删除、安装/启动任务、Microsoft 凭据生命周期、官方皮肤头像合成和本地内容异步用例。
- `rmcl-content` 覆盖本地内容扫描、同名导入改名、Mod 禁用/启用、资源包 `options.txt` 现代/旧格式、损坏选项保护和删除路径安全；Modrinth mock 测试覆盖搜索 facets、版本/文件/依赖解析、API 错误和取消，并验证只有 Mod 的版本查询会按实例加载器过滤。
- `rmcl-app` 覆盖远程文件下载、SHA-1 校验、同名改名、哈希不匹配不发布和 `ContentInstall` 任务快照/终态；`apps/rmcl/src/state.rs` 验证内容安装任务运行期间禁止本地内容操作。
- `rmcl-domain`、`rmcl-app` 和 `rmcl-launch` 覆盖服务器地址、Server List Ping mock、MOTD/图标、超时、图标缓存、Quick Play/legacy 参数和版本边界；真实窗口中的服务器页仍需逐平台验收。
- `rmcl-install` 与 `rmcl-app` 覆盖 Fabric/Quilt 元数据、profile 改写、版本继承合并、loader library 下载、自定义 Vanilla 实例、目录冲突和 `InstanceCreate` 任务终态。
- GPUI 窗口、页面切换、对话框和真实窗口显示已通过真实窗口截图验证；窗口定位改用 macOS `CGWindowID`，避免依赖屏幕坐标或前台应用。
- 2026-10-25 在 macOS ARM64 使用真实 Microsoft 账户完成设备代码登录、Keychain 写入、profile 同步和官方皮肤头像缓存。
- 游戏运行时最小化、游戏退出后恢复的完整链路尚未在真实 Vanilla 启动中验收；Windows 和 Linux 的窗口恢复与系统凭据存储行为仍需逐平台验证。
