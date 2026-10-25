# Minecraft 版本元数据行为

状态：当前核心行为已实现

日期：2026-10-25

## 当前模型

`rmcl-minecraft` 提供以下只读模型和求值逻辑：

- 远程版本列表：`latest`、`versions[]`、版本 JSON URL、可选 SHA-1 和按精确 ID 查找。
- 具体版本 JSON：ID、继承相关字段、主类、下载信息、asset index、Java 版本、libraries 和时间字段。
- modern `arguments.game` / `arguments.jvm` 与 legacy `minecraftArguments` 的分别保留。
- library 的 artifact、classifiers、natives、rules 和 extract 元数据。
- asset index 内容 `AssetCatalog`、asset objects 和经过验证的 `Sha1Hash`。
- OS、归一化架构、feature 规则求值，以及 modern 参数按顺序筛选与展开。

时间戳目前保留原始字符串，不引入不必要的日期转换。未知 JSON 字段被忽略；当前模型用于读取，不承诺原样写回或保留未知字段。

## 规则语义

- 缺失或空 `rules`：允许。
- 非空 `rules`：从拒绝开始，按声明顺序求值，最后一个匹配规则决定结果。
- 缺失 `action`：视为 `allow`。
- `os.name`：接受明确的 `windows`、`osx`、`linux`；未知名称不匹配。
- `os.version` 与 `os.arch`：使用 `regex`，要求正则覆盖整个值。
- 非法正则：返回结构化错误，不静默当作允许。
- features：所有键值都必须匹配；缺失 feature 按 `false` 处理。

OS 版本与架构由调用方显式提供。`Architecture` 将常见别名归一化：`amd64`/`x64` 归为 `x86_64`，`arm64` 归为 `aarch64`，并提供 32/64 位指针宽度；未知架构保留名称但不猜测位数。

## 参数语义

modern 参数支持：

- 直接字符串元素。
- 带 rules 的对象。
- `value` 为字符串或字符串数组。
- `values` 为字符串或字符串数组。

同时提供 `value` 与 `values`、两者都缺失、或使用数字/布尔代替字符串时，解析失败。`evaluate_arguments` 保持声明顺序，只筛选规则和展开数组，不处理 `${...}` 占位符。

legacy 字符串在元数据解析阶段只保留原始内容，不在该阶段切分。legacy tokenizer、占位符替换、默认 JVM 参数和最终启动参数拼接由 [启动计划与游戏进程](../architecture/LAUNCH.md) 实现。

## 与 HMCL 容错行为的有意差异

RMCL 追求功能兼容，不复制所有容错异常：

- 已知字段类型错误时使用 serde 解析错误，不静默把错误类型改成空值。
- feature 缺失按 `false`，不采用“缺失与显式 false 不同”的映射方式。
- 未识别的 OS 名称不匹配，不把未知限制当作无限制。
- 非法 OS 正则返回错误，不根据字段选择静默允许或拒绝。
- 暂不实现 TLauncher、MultiMC、HMCL patches 等扩展字段的私有兼容逻辑。

如果真实元数据证明某项差异影响受支持版本，必须在 notes 记录案例后再修改行为与测试。

## 尚未实现

- `rmcl-minecraft` 本身不做 `inheritsFrom` 递归合并；该逻辑由 `rmcl-install::resolve_version` 实现，见 [版本继承、加载器安装与实例创建](../architecture/LOADERS.md)。
- TAR 等其他归档格式；natives 的 ZIP/JAR 解压已在 [ZIP 安全解压](../architecture/ARCHIVE_EXTRACTION.md) 实现，classifier 选择、`${arch}` 替换与提取计划已在 [Vanilla 安装计划](../architecture/INSTALL_PLAN.md) 实现。
- 内容寻址缓存与修复执行；通用下载和 SHA-1/size 校验已在 [下载执行器](../architecture/DOWNLOAD_EXECUTOR.md) 实现。

因此，当前 crate 是元数据与规则核心；安装、下载和启动分别由对应 crate 负责。
