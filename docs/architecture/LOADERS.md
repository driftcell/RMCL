# 版本继承、加载器安装与实例创建

状态：版本继承合并、Fabric/Quilt 加载器安装、Forge/NeoForge 现代 processor installer 安装、自定义实例创建和从已有实例创建加载器实例已实现

日期：2026-10-26

## 边界

加载器安装由领域规则和应用层编排共同完成：

- `rmcl-install` 的 `resolve_version` 读取版本 JSON 链并合并成独立 profile；`plan_vanilla` 接受已解析的 profile，并允许 `jar` 指向基础版本。
- `rmcl-app` 的 `LoaderCatalog` 获取 Fabric/Quilt/Forge/NeoForge 版本元数据。
- `ForgeInstaller` 下载 Forge/NeoForge installer JAR，安全解压，解析 `install_profile.json`，下载 processor 依赖并执行 Java processor 链。
- `InstanceCreator` 编排“基础版本 → 加载器文件 → 实例文件”的创建过程。
- `TaskManager` 以 `InstanceCreate` 任务持有创建、下载、processor 进度、取消和终态。

加载器元数据客户端和 installer 不读取账号，不写启动器配置，也不依赖 GPUI。

## 版本继承合并

`resolve_version(layout, id)` 从目标版本开始沿 `inheritsFrom` 向上读取，最多 16 层，检测循环和缺失父版本。随后从最老的父版本向子版本合并：

- `id`：子版本。
- `mainClass`、`assetIndex`、`assets`、`javaVersion`、`releaseTime`、`type`：子版本优先，否则继承父版本。
- `libraries`：子版本按 `group:artifact:classifier:@extension` 覆盖父版本同名坐标，其余先父后子。
- `arguments.jvm` / `arguments.game`：先父后子；`minecraftArguments` 子版本优先。
- `downloads`：按 key 合并，子版本覆盖父版本。
- `minimumLauncherVersion` / `complianceLevel`：取较大值。
- `jar`：子版本 `jar` → 父版本 `jar` → 基础版本 ID。
- 返回的 profile `inheritsFrom = None`，可以直接进入计划和启动。

父版本 libraries 的顶层 `sha1` / `size` 会被保留并用于校验；这是 Fabric/Quilt launcher profile 的常见格式。

`plan_vanilla` 不再拒绝 `jar != id`：client JAR 计划到 `jar` 指向的版本目录，因此 loader 实例可以复用基础版本的 client JAR，而不是复制一份。

## Fabric / Quilt 元数据

`LoaderCatalog` 使用以下官方元数据：

- Fabric 版本：`https://meta.fabricmc.net/v2/versions/loader/<game>`
- Fabric profile：`https://meta.fabricmc.net/v2/versions/loader/<game>/<loader>/profile/json`
- Quilt 版本：`https://meta.quiltmc.org/v3/versions/loader/<game>`
- Quilt profile：`https://meta.quiltmc.org/v3/versions/loader/<game>/<loader>/profile/json`

请求带明确 `User-Agent` 和超时，支持取消。返回的 profile JSON 会被校验为对象，然后只改写：

- `id`：目标实例 ID。
- `inheritsFrom`：基础 Minecraft 版本 ID。

其他字段原样保留，包括 mainClass、arguments、libraries 和未知字段。

## Forge / NeoForge 元数据

`LoaderCatalog` 使用以下官方来源：

- Forge 版本：`https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json`
- Forge installer：`https://maven.minecraftforge.net/net/minecraftforge/forge/<game>-<loader>/forge-<game>-<loader>-installer.jar`
- NeoForge 版本：`https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge`
- NeoForge 1.20.1 版本：`https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/forge`
- NeoForge installer：`https://maven.neoforged.net/releases/net/neoforged/{neoforge,forge}/<version>/<artifact>-<version>-installer.jar`

Forge 列表中的 Maven 版本带有 `<game>-` 前缀，界面显示去掉前缀的加载器版本，同时保留完整 `installer_version` 用于拼接 installer URL。NeoForge 1.20.1 使用 `1.20.1-<loader>` 形式并显示去掉前缀的版本；其他版本从 Maven 版本反推 Minecraft 版本（`21.1.80` → 1.21.1、`26.3.0.99` → 26.3，快照 `0.25w14craftmine.3-beta` → 25w14craftmine），只保留匹配当前游戏版本的条目。版本按数字段倒序排列，稳定版优先。

只有 Minecraft 1.13 及以后提供 `spec` 格式 installer；更早的 Forge 使用 legacy installer，版本列表直接返回明确的“不支持 legacy installer”错误，不展示不可用版本。

## Forge / NeoForge installer

`ForgeInstaller` 执行现代 `install_profile.json`（`spec=1`）描述的客户端安装：

1. 下载 installer 到独立临时目录，先读取 Maven 的 `.sha1` sidecar 校验 installer；缺失 sidecar 时容忍，存在但无效或请求失败则中止。随后用现有安全 ZIP 解压器解压；拒绝路径穿越和超限归档。
2. 读取 `install_profile.json`。缺少 `spec` 的 legacy installer 返回 `LoaderInstaller` 错误；`minecraft` 必须等于基础版本。
3. 读取 `json` 指向的内嵌版本 JSON，校验后作为实例 profile 的基础；`inheritsFrom` 仍指向基础版本。
4. 将 installer 内嵌 `maven/` 复制到实例 `libraries/`，其余 install profile libraries 通过 `plan_libraries` 下载和校验。
5. 解析 `data`：
   - `{VAR}` 引用已解析变量；
   - `'literal'` 为字面量；
   - `[group:artifact:version[:classifier][@extension]]` 解析为 `libraries/` 中的路径；
   - 其他相对路径从 installer 解压目录复制到临时 data 目录。
6. 注入内置变量 `SIDE=client`、`MINECRAFT_JAR`、`MINECRAFT_VERSION`、`ROOT`、`INSTALLER`、`LIBRARY_DIR`。
7. 只执行没有 `sides` 或包含 `client` 的 processor，按顺序解析参数、运行 processor JAR 的 `Main-Class`，并校验 `outputs` 中的 SHA-1。
8. `DOWNLOAD_MOJMAPS` processor 不启动 Java，而是按基础版本的 `client_mappings` 下载官方映射，写入 processor 声明的输出路径并校验 SHA-1。
9. 所有 processor 输出都位于实例 `libraries/` 内；输出已存在且哈希匹配时跳过执行，哈希不匹配时删除后重跑。

processor 使用基础版本对应的 Java 要求，由 `JavaService` 选择或安装运行时。processor 在临时 data 目录中运行，避免向启动器工作目录写入文件；stdout/stderr 在失败时截取尾部进入错误信息；取消会终止子进程。

生成的部分 Forge client JAR 无法保证压缩后字节一致，因此 JAR 输出在 SHA-1 不匹配时还会做 ZIP 结构校验；非 JAR 输出必须严格匹配 SHA-1。

## 实例创建流程

`InstanceCreator::create` 接受基础版本、实例 ID、显示名称、平台和可选加载器：

1. 校验实例 ID 和加载器版本；Forge/NeoForge 的显示版本与 installer 版本都不能为空。
2. 如果基础版本 JSON 不存在，先用现有 Vanilla 安装器安装基础版本；如果已存在则复用。Forge/NeoForge 需要真实 client JAR，因此即使基础版本 JSON 已存在，也会以 `VersionSource::Existing` 补全基础文件。基础版本可能作为一个独立实例出现在列表中。
3. 如果加载器为 Fabric/Quilt，获取并改写 loader profile；如果为 Forge/NeoForge，运行 installer processors 并取得内嵌版本 JSON；如果为 Vanilla 自定义实例，则写入最小的 `{id, inheritsFrom}` profile。
4. 写入实例版本 JSON，再写入 `rmcl.instance.toml`，记录显示名称、基础版本、隔离模式和加载器。
5. 使用 `VersionSource::Existing` 调用现有安装器，解析继承链并下载/校验基础文件与 loader libraries。
6. 返回带实例布局的 `InstallOutcome`。

实例目录已存在时拒绝覆盖。manifest 写入失败会删除刚写入的版本 JSON；后续下载失败则保留一个可修复/删除的实例壳。

### 从已有实例添加加载器

实例详情页在没有加载器时提供“添加 Mod 加载器”。RMCL 不原地改写已有实例的版本 JSON，因为基础版本 JSON 可能同时被其他实例继承，也可能是用户已经使用的 Vanilla 实例。该操作复用实例创建流程：

1. 读取源实例 manifest，要求 manifest 完整、没有加载器，并用 manifest 的 `minecraft_version` 作为基础版本。
2. 校验基础版本 JSON 是完整 Vanilla profile，且 id 与基础版本一致；拒绝以加载器或其他子 profile 作为基础。
3. 按加载器版本生成新的实例 ID 和显示名称，安装加载器并创建隔离实例。
4. 将源实例游戏目录中的内容复制到新实例。隔离实例复制除版本 JSON/JAR、natives、日志、崩溃报告、下载缓存和 manifest 之外的顶层内容；共享实例复制 `mods/`、资源包、光影、存档、`config/` 等常用内容以及 `options.txt`、`servers.dat`。
5. 源实例保持不变；复制失败时删除刚创建的目标实例。

该流程不会升级、更换或移除已有加载器，也不修改基础版本实例本身。

### Vanilla 自定义实例

选择 Vanilla 且实例 ID 不同于基础版本时，会创建一个继承基础版本的最小 profile。这样同一基础版本可以有多个隔离实例，而不复制 client JAR、libraries 或 assets。

如果实例 ID 就是基础版本 ID，则走普通 Vanilla 安装路径，不覆盖基础版本 JSON。

## 任务与界面

新增 `TaskKind::InstanceCreate`、`TaskPhase::InstanceCreate` 和 `TaskSubject::Instance { id, name }`。任务按实例 ID 互斥，创建过程中显示基础安装、加载器获取、实例写入、加载器处理器和实例文件安装阶段。Forge/NeoForge processor 进度以 `TaskProgress::Processors` 暴露当前 processor、完成数和总数。

实例页的“新建实例”进入创建页：

- 输入 Minecraft 版本 ID。
- 选择 Vanilla / Fabric / Quilt / Forge / NeoForge。
- 加载器页获取并选择加载器版本，稳定版默认优先。
- 显示名称和实例 ID 可留空；ID 按基础版本和加载器自动生成，Forge/NeoForge 形如 `forge-<loader>-<base>`、`neoforge-<loader>-<base>`。
- 创建完成后自动选中实例并进入实例详情。

## 当前限制

- 只支持 `spec` 格式的 Forge/NeoForge installer；Minecraft 1.12.2 及更早的 legacy Forge installer 明确不支持。
- 不支持原地给已有实例追加、升级或移除加载器；实例详情页的“添加 Mod 加载器”会创建新的加载器实例并复制现有内容。
- 不自动安装 Fabric API / Quilt API / Forge 配套 Mod。
- 不校验加载器版本与基础版本的完整兼容矩阵；以元数据返回的版本列表为准。
- 基础版本可能作为独立实例出现；尚未隐藏纯依赖实例。
- 真实 Forge/NeoForge 启动只验证到 ModLauncher 初始化日志，尚未做逐平台主菜单、世界加载和游戏内验收。

## 验证

- `rmcl-install` 覆盖父子字段合并、libraries 顺序、arguments 顺序、jar 解析、循环、缺失父版本、最大深度、loader 风格顶层哈希和空 URL 生成文件。
- `rmcl-minecraft` 覆盖 Fabric/Quilt 风格库的顶层 `sha1`/`size` 解析。
- `rmcl-app` 覆盖 Fabric/Quilt/Forge/NeoForge 元数据与 installer URL、profile 改写、基础版本复用、loader library 下载、自定义 Vanilla 实例、目录冲突和 `InstanceCreate` 任务快照。
- `forge_install` 使用 mock installer 和 fake Java 覆盖 `DOWNLOAD_MOJMAPS`、processor 执行、输出校验、data 文件复制、installer SHA-1 不匹配和 legacy installer 拒绝；仅在 Unix 运行。
- 标记为 `#[ignore]` 的真实网络 smoke 使用官方 Minecraft 1.21.1 client JAR 和真实 installer，运行完整 Forge `1.21.1-52.1.20`、NeoForge `21.1.80` processor 链，并确认生成的 client JAR 是有效 ZIP。2026-10-25 两者均在本机通过。
- 同一 smoke 在 macOS ARM64 使用真实 Java 启动安装后的实例，日志进入 ModLauncher 初始化并出现对应加载器标记后主动终止；这验证了安装结果、版本继承和启动参数，但不等于主菜单或游戏内验收。
- 2026-10-25 对真实 Fabric/Quilt/Forge/NeoForge 元数据 endpoint 做了只读 smoke。
