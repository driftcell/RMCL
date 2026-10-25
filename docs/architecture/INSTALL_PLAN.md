# Vanilla 安装计划

状态：当前核心行为已实现

日期：2026-10-25

## API 与阶段

`rmcl-install` 提供两个纯规划入口：

- `plan_vanilla`：client JAR、普通 libraries、native libraries 和 asset index。
- `plan_asset_objects`：下载并解析 asset index 后，根据 `AssetCatalog` 生成 content-addressed objects 计划。

两阶段分离允许执行器先取得 asset index，再规划其中的对象。当前不会读写文件、访问网络、修改版本 JSON 或执行解压。

`InstallPlan` 包含：

- `files`：文件类型、目标路径、候选 URL、可选 SHA-1 和 size。
- `extractions`：native JAR 目标、解压目录和 `extract.exclude` 前缀。

## Library 解析

- rules 不适用的条目完全跳过。
- 普通条目生成 `Library` 文件，供后续 classpath 使用。
- 现代显式 classifier 的 LWJGL native JAR 没有结构化 `natives` 时仍按普通 `Library` 处理，不自动解压。
- 带 `natives` 的条目只生成匹配当前 OS 的 `NativeLibrary` 与提取计划，不额外下载基础 artifact。
- `natives` 没有当前 OS key 时跳过整个 native 条目，不回退到基础 JAR。
- `${arch}` 按目标平台的 JVM 指针宽度替换为 `32` 或 `64`；未知架构和未解析占位符报错。

`downloads` 整体缺失时允许旧式 Maven 回退。一旦提供 `downloads`，就要求选中的 native classifier 存在于 `downloads.classifiers`，否则返回 `MissingNativeClassifier`，不静默合成矛盾元数据。

## 路径与 URL 优先级

Maven 坐标支持 `group:artifact:version[:classifier][@extension]`，默认扩展名为 `jar`。各组件必须非空且安全。

每个选中的下载项按以下优先级解析：

- 路径：`downloads.*.path` 优先，否则按 Maven 坐标推导。
- URL：`downloads.*.url` 优先，否则使用 `library.url` 作为 base，最后使用 `https://libraries.minecraft.net/`。
- base URL 与路径按路径段组合并编码，不进行不透明的字符串替换。

只接受绝对 `http`/`https` URL。显式 URL 不会无条件与 base URL 组合成额外候选；镜像候选以后必须由明确的来源策略提供。

## 完整性、去重与冲突

- SHA-1 字符串通过 `hex` 解析为 20 字节值；接受大小写并统一为小写表示。
- size 使用 `u64`，缺失 SHA-1 不等于校验通过。
- 相同目标且 SHA-1/size 一致的重复项去重，并合并不同 URL。
- 没有 SHA-1 时，不为同一目标合并无法证明内容一致的不同 URL。
- 相同目标但 SHA-1 或 size 冲突时报错。
- asset objects 按 hash 去重；同 hash 但 size 不一致时报错。
- asset index 使用 `assetIndex.id`，声明的 SHA-1/size 均进入计划，不采用“URL 是否包含 hash”的校验启发式。

## 明确非目标

当前尚未实现：

- 真实下载、缓存、重试、取消和原子落盘。
- 实际 ZIP/JAR 解压及 ZIP slip、symlink 防护。
- `inheritsFrom`、patches 和 `jar != id` 的 client 重定向。
- 无 `assetIndex` 的旧版本内置回退表。
- `virtual` / `map_to_resources` 资产名称重建。
- 镜像选择、Java 选择、启动参数与修复执行。

这些非目标不能通过把计划生成成功解释为安装或启动成功。
