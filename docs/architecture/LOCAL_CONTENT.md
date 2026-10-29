# 本地内容管理

状态：实例级 Mod、资源包和光影的本地扫描、导入、删除与启用/禁用已实现；资源包 `pack.mcmeta`/图标解析、格式兼容判断、优先级调整、光影包结构校验和加载器类型化检测已提供核心 API，Mod JAR 元数据索引供安装预检使用；Modrinth 远程下载、推荐和必需依赖见 [远程内容与 Modrinth](REMOTE_CONTENT.md)，CurseForge、可选依赖自动选择和整合包尚未实现

日期：2026-10-29

## 边界

`rmcl-content` 管理实例内容目录中的本地文件，不访问网络、不读取启动器配置、不依赖 GPUI。异步包装位于 `rmcl-app`，界面只消费扫描结果和操作结果。

内容目录来自 [`InstanceLayout`](INSTANCES.md)：

| 类型 | 目录 | 接受的文件 | 接受文件夹 |
| --- | --- | --- | --- |
| Mod | `mods/` | `.jar`、`.zip`、`.litemod`，可带 `.disabled` | 否 |
| 资源包 | `resourcepacks/` | `.zip` | 是，要求根目录有 `pack.mcmeta` |
| 光影包 | `shaderpacks/` | `.zip` | 是 |

隔离实例使用实例目录下的内容目录；共享实例使用 `.minecraft` 根下的全局目录。内容管理不会跨实例读取或写入。

## 模型

- `ContentKind`：`Mod`、`ResourcePack`、`ShaderPack`。
- `LocalContent`：类型、安全相对路径、绝对路径、大小、启用状态和是否为文件夹。
- `ContentLibrary`：一个实例的全部条目和非致命警告。
- `ContentWarning`：符号链接、无效名称、不可读取条目、`options.txt` 错误或整类扫描失败。

扫描不会因为一个损坏条目而隐藏整个列表；无法读取的条目进入警告。内容目录不存在时按空目录处理。

## 包元数据与兼容性

- `read_resource_pack_metadata` 同时支持 ZIP 和文件夹资源包，读取 `pack.mcmeta` 中的 `pack_format`、`supported_formats`（对象或数组）、`min_format`/`max_format` 回退和描述文本组件；支持 1.21.9+ 的 `[major, minor]` 格式对，描述中的 `§` 格式码会转为纯文本，`pack.png` 以受大小限制的原始字节返回。
- `ResourcePackMetadata::compatibility` 与目标资源格式比较；目标格式未知时返回 `Unknown`，不猜测兼容性。
- 目标资源格式由 `read_client_resource_pack_format` 从 client JAR 的 `version.json` 读取：现代客户端使用 `pack_version.resource_major` 和 `resource_minor`，旧客户端支持数字 `pack_version`。`rmcl-app::resource_pack_format` 会沿加载器实例的 `jar` 或 `inherits_from` 找到基础 client JAR。
- `read_shader_pack_metadata` 校验 `shaders/` 目录并读取可选的 `shaders/pack.json` 描述；不执行 shader 代码。
- `ShaderLoader` 将 Iris、Oculus、OptiFine 类型化；只有已启用的加载器 Mod 才算光影加载器，`.disabled` 不计入。
- `move_resource_pack` 在 `options.txt` 的 `resourcePacks` 列表中移动已启用资源包；数组末尾优先级更高，未启用资源包返回结构化错误。

## 扫描规则

- Mods 扫描根目录和一层子目录；跳过隐藏文件/目录、`.connector` 和符号链接。子目录中的 Mod 保留相对路径，例如 `subdir/mod.jar`。
- 资源包扫描根目录的 `.zip` 文件和包含 `pack.mcmeta` 的文件夹；启用状态来自实例 `options.txt`。
- 光影包扫描根目录的 `.zip` 文件和文件夹；没有统一的启用/禁用开关，因此始终显示为已安装。
- 以 `.disabled` 结尾的 Mod 仍会显示，展示名去掉后缀。
- 符号链接不跟随，也不能通过内容 API 删除或切换。

## 导入

`import_file` 只接受用户选择的普通文件，拒绝符号链接、目录和不支持的扩展名。

- `ConflictPolicy::Rename`：默认策略，按 `name (1).ext` 生成不冲突的名称；比较不区分大小写，避免 Windows/macOS 大小写不敏感文件系统覆盖。
- `ConflictPolicy::Reject`：目标名称已存在时返回 `ContentError::Conflict`。
- 内容先写入同目录临时文件，`sync` 后用 `persist_noclobber` 发布；失败不会留下半个文件，也不会覆盖已有内容。
- 导入的 Mod 按文件名决定启用状态；资源包默认禁用，等待用户启用；光影包直接显示为已安装。

导入复制源文件内容，源文件保留原位置；不跟随源文件符号链接。目标内容目录本身是符号链接时拒绝导入。

## 启用与禁用

### Mods

使用 Minecraft 社区惯例：

- 禁用：`example.jar` → `example.jar.disabled`
- 启用：`example.jar.disabled` → `example.jar`

目标文件已存在时返回冲突，不覆盖。嵌套 Mod 在原目录内改名。

### 资源包

资源包通过实例的 `options.txt` 控制：

- `resourcePacks` 是启用列表，启用的包排在末尾，因此优先级最高。
- `incompatibleResourcePacks` 在禁用或重新启用时同步清理。
- 现代版本使用 `file/<名称>` 条目，1.12 及更早使用纯名称。已有条目决定格式；列表为空时按实例基础版本推断。
- 只修改这两个键，保留其他 `options.txt` 行；通过临时文件和原子替换写回。
- `options.txt` 列表不是合法 JSON 字符串数组时，扫描产生警告并把资源包显示为禁用；启用/禁用操作返回错误，不重写文件。

### 光影包

光影包没有原版统一的启用状态。RMCL 只管理文件和文件夹，不修改 Iris/OptiFine 配置。

## 删除

删除前校验相对路径、每一级父目录和符号链接：

- 拒绝符号链接祖先和目标本身。
- Mod 目录不能通过内容 API 删除；Mod 只删除文件。
- 资源包删除前先从 `options.txt` 移除；选项文件损坏时阻止删除。
- 文件夹资源包和光影文件夹使用递归删除。

删除只作用于目标内容条目，不删除实例、版本 JSON、共享 libraries 或 assets。

## 界面

实例详情页提供：

- Mods / 资源包 / 光影包三个标签和数量。
- 搜索、导入文件、打开内容目录和重新扫描。
- 文件名、相对路径、大小/文件夹、启用状态。
- Mod 和资源包的启用/禁用按钮，以及所有条目的删除确认。
- 扫描警告和错误横幅；导入部分失败时汇总成功和失败数量。

导入按钮统一使用 GPUI 的异步路径选择接口。不能从点击回调里同步调用 `NSOpenPanel::runModal`：嵌套的原生 modal 循环会在 GPUI 仍持有 `App` 可变借用时处理定时器，触发 `RefCell already borrowed` panic。文件操作通过应用层 Tokio runtime 的 `spawn_blocking` 执行，不阻塞 UI 线程。

## 当前限制

- Modrinth 远程下载已由 [远程内容与 Modrinth](REMOTE_CONTENT.md) 提供；CurseForge 尚未接入。
- 本地内容列表仍只显示文件名、路径和大小；安装预检会解析 Fabric/Quilt/Forge/NeoForge Mod 元数据，见 [远程内容与 Modrinth](REMOTE_CONTENT.md)，但列表本身尚未展示真实名称、图标或版本。
- 尚未计算本地文件哈希，因此不能做远程识别或更新检查。
- `pack_format` 兼容判断和资源包元数据 API 已提供，但尚未接入 GPUI 列表展示、导入/下载后的兼容性提示和 `incompatibleResourcePacks` 自动修正。
- 资源包启用格式的版本推断是启发式；空列表的旧版本可能被写成现代 `file/` 格式，需要后续按精确版本表修正。
- 光影包目前只校验结构和加载器状态；尚未读写 Iris/OptiFine 的启用配置，也没有界面级“当前光影”切换。
- 尚未支持从文件夹导入资源包/光影，只管理已存在的文件夹。
- 尚未实现内容更新、批量启用、拖放导入和整合包安装。
- 加载器安装见 [版本继承、加载器安装与实例创建](LOADERS.md)；当前支持 Fabric/Quilt 和现代 Forge/NeoForge installer。

## 验证

- `rmcl-content` 覆盖 Mod 根目录/一层子目录/禁用项、资源包文件和文件夹、光影文件/文件夹、符号链接跳过、同名改名、拒绝覆盖、Mod 切换、`options.txt` 现代/旧格式、损坏选项不重写、删除与目录删除限制。
- 新增覆盖资源包 ZIP/文件夹元数据、`supported_formats` 对象/数组、`min_format`/`max_format` 回退、元数据大小限制、client JAR `pack_version`、光影包结构与描述、资源包优先级移动，以及禁用光影加载器不计入检测。
- `rmcl-app` 集成测试覆盖 `spawn_blocking` 包装下的导入、扫描、禁用和删除链路。
- `apps/rmcl` 已编译并通过现有状态测试；真实窗口中的文件选择、标签切换和导入/删除交互仍需在 macOS、Windows、Linux 逐平台验收。
