# 应用层 Vanilla 安装与启动

状态：`rmcl-app` 安装、安装任务、Java ensure 和游戏任务已实现

日期：2026-10-25

## 边界

`rmcl-app` 组合现有元数据、安装计划、下载器、安全解压、Java 和启动 API，不重复这些 crate 的领域规则，也不依赖 GPUI。`VanillaInstaller`、`JavaService`、`GameLauncher` 可独立调用，`TaskManager` 在它们之上提供应用级生命周期。

实例内容的本地扫描与变更由 `rmcl-content` 提供，`rmcl-app` 只提供 `spawn_blocking` 异步包装；见 [本地内容管理](LOCAL_CONTENT.md)。远程内容下载会另外接入任务模型，不复用当前只负责本地文件的同步用例。

`VanillaInstallRequest` 要求调用方提供 `.minecraft` 根、精确版本 ID、目标平台和 `VersionSource`：

- `Manifest`：从配置的版本 manifest 查找精确 ID；官方构造器使用 Mojang `version_manifest_v2.json`。
- `LocalJson`：读取本地版本 JSON，先验证 JSON 和版本 ID，再保存原始字节。
- `Existing`：读取仓库中已有的版本 JSON，不访问版本 manifest。

`VanillaSources` 允许注入 manifest 和 asset object 来源根，生产默认与本地 mock 使用同一执行路径。自定义 asset base 不降低 SHA-1/size 校验，也不是完整镜像策略。

仓库根在任务开始转为绝对路径。直接使用 `VanillaInstaller` 时，调用方必须串行化同一仓库和版本的安装；`TaskManager` 则内置下述活动任务互斥。

## 版本元数据

- manifest 通过 `rmcl-net` 下载到临时目录，复用超时、重试、取消和脱敏错误，不新增 HTTP 客户端。
- 远程版本 JSON 直接下载到标准版本目录；manifest summary 中的 SHA-1 进入下载校验，已验证文件可以跳过。
- 本地 JSON 在发布前完成 UTF-8、serde 和请求 ID 校验；内容变化时使用同目录临时文件、sync 和 persist，原样保留未知字段与格式。
- `Existing` 使用独立的读取阶段，不伪装成写入。
- 版本 JSON 可以提前存在，但它不是安装完成标记；后续 client、libraries 和 assets 仍按计划分别校验。

暂不支持 `inheritsFrom`、`virtual: true` 或 `map_to_resources: true`，这些情况返回结构化错误，不按不完整 Vanilla 继续安装。

## 实例解析与隔离

`VanillaInstaller` 在解析版本 JSON 后确定实例布局：

- 版本 JSON 原本不存在且来源不是 `Existing`：写入 `rmcl.instance.toml`，默认 `isolation = "isolated"`，并创建标准内容目录。
- manifest 已存在：按其中的 `isolation` 决定游戏目录，不覆盖已有名称或加载器字段。
- 版本 JSON 已存在但没有 manifest：保持 legacy 共享模式；重装不会静默把用户已有的全局内容移出视野。
- `Existing` 修复：只读取 manifest，不创建或改写。

`InstallOutcome` 同时返回 `MinecraftLayout` 和解析后的 `InstanceLayout`。后者提供游戏目录、`mods/`、`resourcepacks/`、`shaderpacks/`、`saves/` 和 manifest 路径。启动使用实例布局；共享 libraries 和 assets 仍由仓库布局提供。

旧实例迁移由 `migrate_instance_to_isolated` 提供：复制全局内容中实例尚不存在的文件，不覆盖、不跟随符号链接，最后写入 `isolation = "isolated"` 的 manifest；失败时实例保持共享。详见 [实例与版本隔离](INSTANCES.md)。

## 执行顺序与副作用

1. 解析、读取或保存版本 JSON；新安装同时建立实例 manifest。
2. 生成并下载 base 计划：client、普通 libraries、native libraries 和 asset index。
3. 读取 asset index，生成并下载 asset objects 计划。
4. 在一个 `spawn_blocking` 中按顺序调用 `merge_zip`，合并全部 native JAR。
5. 返回带实例布局的 `InstallOutcome`。

base 失败时不会下载 assets 或提取 natives；assets 失败时不会提取 natives。已经原子发布的文件不会因后续失败或取消而自动撤回。native merge 保留已有文件，不是损坏修复或过期文件清理。

`InstallOutcome` 保留绝对 `MinecraftLayout`、解析后的 `InstanceLayout`、目标平台、`GameVersion`、base/asset 计划和 native 报告，供后续启动复用。`DownloadSummary` 区分下载与跳过的文件数和字节数，只统计 base 与 asset objects，不包含版本 manifest 或版本 JSON。

## 进度、取消与错误

`InstallPhase` 区分版本解析、读取、保存、base/assets 规划与下载、asset index 读取、native 合并和完成。

- `InstallEvent` 通过有界 Tokio mpsc 通道发送；接收方必须持续消费，事件接收端关闭不会使安装失败。
- 下载事件携带阶段、当前目标、当前文件字节进度、可选总量和终态文件数。终态文件数包含完成、跳过、失败和取消，不能解释为成功数量。
- native 事件按“已合并 JAR 数/总数”报告；不伪造需要预扫描 ZIP 才能获得的全局字节百分比。
- `CancellationToken` 贯穿单文件下载、批量下载、阶段边界和 native 解压。请求取消与最终 `AppError::Cancelled` 分开，取消不映射为普通失败。
- 批量错误只保留目标和脱敏消息，不把原始 `DownloadRequest` 或可能带 query 的 URL 放入应用错误。

高频事件的界面合并由后续 UI 桥接负责；安装任务快照和广播由 `TaskManager` 提供。

## 任务管理

`TaskManager::start_install`、`start_launch` 和 `start_launch_installed` 在应用级 Tokio 任务中持有对应 future，并暴露以下语义：

- `TaskId` 在进程内单调分配，不随阶段变化，也不复用已结束 ID；当前不持久化。
- 活动任务按“绝对化仓库根 + 版本 ID”互斥，不同平台目标也不能并发写同一版本的共享文件。重复请求返回已有任务 ID 对应的 `TaskAlreadyRunning`。
- `TaskSnapshot` 包含任务类型、统一 `TaskPhase`、`Running`/`Cancelling`/`Completed`/`Cancelled`/`Failed`/`Exited`、进度、脱敏错误、可选退出状态、停止标记和累计游戏日志行数。
- `cancel` 先发布 `Cancelling` 快照，再触发同一个协作式取消令牌。安装或 spawn 前取消最终为 `Cancelled`；游戏运行中取消会 kill 直接子进程、继续排空日志，最终以 `Exited` 和 `stop_requested` 返回。终态任务上的取消是 no-op。
- `TaskEvent` 使用有界 broadcast。慢订阅者可能收到 `Lagged` 并丢失中间快照或日志事件，可以随时调用 `snapshot` 和 `logs` 读取当前状态；订阅者关闭不会取消任务。
- 终态快照保留到 `dismiss`，dismiss 后发送移除事件；活动任务不能 dismiss。
- `TaskManager` 和其克隆共享注册表，临时 View 可以只持有订阅，不拥有安装任务。

当前互斥不解决不同版本之间的所有共享 asset/library 并发写入，也不处理外部进程或符号链接别名指向同一仓库的情况。

## Java ensure

`JavaService` 组合现有 Java API，不重新推导默认 major、component 或回退顺序：

1. `JavaDetector` 对显式 managed roots、RMCL store、`JAVA_HOME` 和 PATH 做真实 probe。
2. `select_java` 返回满足 `JavaRequirement` 的候选时，结果为 `Detected`，不发起安装。
3. 没有候选且 `auto_install` 关闭时返回 `JavaNotFound` 和逐候选探测错误。
4. 允许自动安装时发出 `Installing` 事件，并调用 `MojangRuntimeInstaller`；返回的 runtime 已经过下载、校验、发布和 probe。
5. 后续 ensure 能通过修正后的 detector 直接发现 store 中的 runtime，不要求再次请求 index。

`JavaServiceConfig` 要求显式 store 根，生产默认只允许官方 runtime URL；测试可使用本地 index 和 `AnyHttp`。Java 安装当前只有阶段事件，没有字节级进度。探测和 runtime 元数据获取不是完全可取消的，取消可能在当前操作返回后才被观察，调用方不能把请求取消当成已经停止。

## 游戏启动与日志

`GameLauncher::run` 按以下顺序拥有完整启动过程：

1. 使用 `VanillaInstaller` 完成安装并保留 `InstallOutcome`。
2. 从版本与平台构造唯一的 `JavaRequirement`，调用 `JavaService::ensure`。
3. 从离线账号生成新的启动 token，使用安装结果构造 `LaunchOptions` 和 `LaunchPlan`。
4. 发出 `Starting` 后创建 `GameProcess`，随后发出 `Started` 并持续转发 stdout/stderr。
5. 等待自然退出或取消后的 kill，排空日志后返回 `GameLaunchOutcome`。

`GameLauncher::run_installed` 跳过第 1 步：读取已有版本 JSON、生成计划、检查 base 文件和 natives 目录存在，然后执行第 2 至第 5 步；不检查 asset objects 的哈希。缺少基础文件时返回 `InstallationIncomplete`，由界面引导用户先执行检查修复。`Started` 只表示 JVM 进程创建成功，不表示主类加载、窗口或主菜单已经就绪。游戏自身以非零码退出仍返回正常 `GameLaunchOutcome`，任务层记录为 `Exited`；安装、Java、计划、spawn 或进程 I/O 错误才记录为 `Failed`。当前不自动推断崩溃原因。

`TaskManager::with_java` 启用 `start_launch` 和 `start_launch_installed`；安装专用构造器没有 Java 服务时会返回 `JavaNotConfigured`。`start_launch` 走完整 ensure/repair；`start_launch_installed` 只读取已有版本 JSON、生成计划并检查 base 文件和 natives 目录存在，不下载也不重新哈希 asset objects。缺少基础文件时返回 `InstallationIncomplete`，由界面提示先修复。游戏任务在整个运行期间保留版本互斥，防止同时重装相同版本。退出后互斥释放。

每个游戏任务保留最近 1024 行已脱敏日志，`log_lines` 记录实际接收的总行数，`logs` 返回当前保留内容。日志事件广播不反压游戏进程，慢订阅者丢失事件不代表这些行超出环形缓冲。应用任务而非 View 持有 `GameProcess`；不承诺启动器退出后 detach 或进程树管理。

大型安装/启动 future 在组合任务中显式 heap-pin，避免 worker 栈随嵌套编排增长；这不改变事件顺序或取消语义。

## 当前不包含

- Microsoft 登录、专用 Quick Play 配置和账号令牌存储
- 实例级 Java、启动参数、内存和窗口设置，以及实例复制、导入导出和重命名
- 版本继承、特殊资产名称重建、ModLoader 和镜像策略
- 完整修复、native 清理、内容寻址缓存和断点续传
- 核心侧日志归档、崩溃原因分析、进程树管理和启动器退出后的 detach
- GPUI 页面或窗口生命周期（由 `apps/rmcl` 持有；`rmcl-app` 仍不依赖 GPUI）

启动器偏好和账户元数据由 `rmcl-store` 持久化，见 [启动器配置](CONFIGURATION.md)。`rmcl-app` 不读写该配置；Microsoft 令牌由系统凭据存储管理，见 [正版账号、令牌与皮肤](AUTHENTICATION.md)。

本地测试使用 mock 和假 JVM 覆盖远程 manifest、两阶段下载、两个 native JAR、重复安装复用、快速启动跳过安装、缺失基础文件、任务互斥、取消、Java 自动安装、日志脱敏、自然非零退出、停止和有界日志；这些测试不访问外部服务。

2026-10-25 使用官方 manifest 和已有真实仓库缓存验证了 `VanillaInstaller::official`：`26.3` 的 5229 个 base/asset 文件全部校验并跳过，`1.7.10` 的 708 个文件全部校验并跳过且 3 个 native JAR 完成合并。该结果验证官方来源解析与复用，不把缓存 smoke 描述为全新下载；新文件下载由上述本地 mock 覆盖。

同日在 macOS ARM64、Temurin 25.0.5 上通过 `TaskManager::start_launch` 运行真实 Vanilla `26.3`：任务依次报告安装、Java 检查、启动和运行阶段，创建 Cocoa 窗口，运行 12 秒后由 smoke 调用 `cancel`，最终得到 `Exited`、`stop_requested = true` 和 45 行保留日志。主动 kill 产生的 signal 9 不是崩溃。随后使用真实 Microsoft 账户和正版 access token 完成了一次真实 Vanilla 启动，用户确认游戏正常进入运行阶段。Windows 和 Linux 的真实游戏启动仍需分别验收。
