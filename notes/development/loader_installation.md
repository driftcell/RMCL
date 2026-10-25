# 加载器安装与实例创建取舍

日期：2026-10-25

状态：Fabric/Quilt/Forge/NeoForge 已形成实现决策，最终规范见 [版本继承、加载器安装与实例创建](../../docs/architecture/LOADERS.md)

## 背景

在线内容下载已经可用，但 Mod 只有在实例安装了加载器后才会生效。需要同时解决：

- 版本 JSON 的 `inheritsFrom` 合并和 `jar` 重定向。
- Fabric/Quilt 版本列表与 launcher profile。
- 新实例向导中的加载器选择。
- 基础版本、loader libraries、实例 manifest 和任务进度的事务关系。

## 参考行为

HMCL：

- 加载器以 patch/component 形式合并到实例 manifest，再重建独立启动 profile。
- Fabric/Quilt 使用元数据 profile，不运行 installer JAR。
- Forge/NeoForge 下载 installer JAR 并执行 processors。
- 新实例向导先选游戏版本，再选加载器版本，最后一次性构建。
- 加载器库的完整性来自 profile 的顶层 sha1/size 或 downloads 字段。

## 决策

- 先实现版本继承合并和 `jar` 重定向，这是所有加载器 profile 的前置。
- 采用“基础版本 → loader profile → 现有安装器”的分步编排，而不是复制 HMCL 的 patch 模型；现有 `VanillaInstaller` 继续负责文件计划、下载和校验。
- Fabric/Quilt profile 直接取官方 launcher profile JSON，只改写 `id` 和 `inheritsFrom`，保留未知字段。
- 自定义 Vanilla 实例使用最小 `{id, inheritsFrom}` profile，支持同一基础版本的多个隔离实例。
- 基础版本不存在时先安装；已存在时复用。当前接受基础版本作为独立实例出现，后续再考虑隐藏纯依赖实例。
- 实例 manifest 在最终文件安装前写入；失败保留可修复实例壳，而不是留下无主目录。
- 加载器任务使用独立 `InstanceCreate` 类型和 `Instance` subject，不复用 Vanilla 安装的 UI 语义。
- Forge/NeoForge 独立实现现代 installer processors，不执行官方 installer 的 Swing UI，也不依赖外部安装器进程。installer JAR 只作为数据来源和 processor 库来源。
- 只接受 `spec` 格式 install profile；legacy Forge installer 明确返回错误。Minecraft 1.13 以下的 Forge 版本不出现在可选列表。
- processor 输出必须落在实例 `libraries/` 内；非 JAR 输出严格校验 SHA-1，JAR 允许 ZIP 结构校验作为压缩差异回退。
- `DOWNLOAD_MOJMAPS` 直接使用基础版本的 `client_mappings`，避免为同一官方文件运行外部 Java 工具。
- Forge/NeoForge 创建实例前确保基础 client JAR 完整，因为 processor 需要读取它；这比 Fabric/Quilt 仅依赖 JSON 更严格。

## 暂缓

- 给已有实例追加/升级/移除加载器。
- legacy Forge installer、OptiFine、LiteLoader、Cleanroom。
- Fabric API / Quilt API 自动安装。
- 依赖实例隐藏、实例重命名和复制。
- 真实加载器游戏启动验收；当前用真实 Minecraft 1.21.1 和官方 installer 跑通 Forge 1.21.1-52.1.20、NeoForge 21.1.80 的完整 processor 链，但尚未启动游戏。
