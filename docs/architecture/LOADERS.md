# 版本继承、Fabric/Quilt 与实例创建

状态：版本继承合并、Fabric/Quilt 加载器安装和自定义实例创建已实现；Forge/NeoForge 尚未实现

日期：2026-10-25

## 边界

加载器安装属于 `rmcl-install` 的领域规则：

- `resolve_version` 读取版本 JSON 链并合并成独立 profile。
- `plan_vanilla` 接受已解析的 profile，并允许 `jar` 指向基础版本。
- `rmcl-app` 的 `LoaderCatalog` 获取 Fabric/Quilt 元数据，`InstanceCreator` 编排“基础版本 → loader profile → 实例文件”的创建过程。
- `TaskManager` 以 `InstanceCreate` 任务持有创建、下载、取消和终态。

加载器元数据客户端不读取账号，不写启动器配置，也不依赖 GPUI。

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

## 实例创建流程

`InstanceCreator::create` 接受基础版本、实例 ID、显示名称、平台和可选加载器：

1. 校验实例 ID、加载器类型和加载器版本；Forge/NeoForge 返回“尚未实现”。
2. 如果基础版本 JSON 不存在，先用现有 Vanilla 安装器安装基础版本；如果已存在则复用。基础版本可能作为一个独立实例出现在列表中。
3. 如果加载器为 Fabric/Quilt，获取并改写 loader profile；如果为 Vanilla 自定义实例，则写入最小的 `{id, inheritsFrom}` profile。
4. 写入实例版本 JSON，再写入 `rmcl.instance.toml`，记录显示名称、基础版本、隔离模式和加载器。
5. 使用 `VersionSource::Existing` 调用现有安装器，解析继承链并下载/校验基础文件与 loader libraries。
6. 返回带实例布局的 `InstallOutcome`。

实例目录已存在时拒绝覆盖。manifest 写入失败会删除刚写入的版本 JSON；后续下载失败则保留一个可修复/删除的实例壳。

### Vanilla 自定义实例

选择 Vanilla 且实例 ID 不同于基础版本时，会创建一个继承基础版本的最小 profile。这样同一基础版本可以有多个隔离实例，而不复制 client JAR、libraries 或 assets。

如果实例 ID 就是基础版本 ID，则走普通 Vanilla 安装路径，不覆盖基础版本 JSON。

## 任务与界面

新增 `TaskKind::InstanceCreate`、`TaskPhase::InstanceCreate` 和 `TaskSubject::Instance { id, name }`。任务按实例 ID 互斥，创建过程中显示基础安装、加载器获取、实例写入和实例文件安装阶段。

实例页的“新建实例”进入创建页：

- 输入 Minecraft 版本 ID。
- 选择 Vanilla / Fabric / Quilt。
- Fabric/Quilt 获取并选择加载器版本，稳定版默认优先。
- 显示名称和实例 ID 可留空；ID 按基础版本和加载器自动生成。
- 创建完成后自动选中实例并进入实例详情。

## 当前限制

- Forge/NeoForge 需要 installer JAR、`install_profile.json` 和 processors，尚未实现。
- 不支持给已有实例追加、升级或移除加载器；加载器只在创建实例时安装。
- 不自动安装 Fabric API / Quilt API 等加载器配套 Mod。
- 不校验加载器版本与基础版本的完整兼容矩阵；以元数据返回的版本列表为准。
- 基础版本可能作为独立实例出现；尚未隐藏纯依赖实例。
- 没有真实 Fabric/Quilt 游戏启动验收；当前验证覆盖 profile 合并、库下载和任务终态。

## 验证

- `rmcl-install` 覆盖父子字段合并、libraries 顺序、arguments 顺序、jar 解析、循环、缺失父版本、最大深度和 loader 风格顶层哈希。
- `rmcl-minecraft` 覆盖 Fabric/Quilt 风格库的顶层 `sha1`/`size` 解析。
- `rmcl-app` 覆盖 Fabric/Quilt 元数据、profile 改写、基础版本复用、loader library 下载、自定义 Vanilla 实例、目录冲突和 `InstanceCreate` 任务快照。
- 2026-10-25 对真实 Fabric/Quilt 元数据 endpoint 做了只读 smoke，确认版本列表和 profile JSON 字段；未执行真实加载器游戏启动。
