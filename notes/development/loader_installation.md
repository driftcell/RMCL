# 加载器安装与实例创建取舍

日期：2026-10-25

状态：已形成实现决策，最终规范见 [版本继承、Fabric/Quilt 与实例创建](../../docs/architecture/LOADERS.md)

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
- Forge/NeoForge 暂不在本切片内；它们的 installer processors 和 Java 执行需要单独设计和验证。

## 暂缓

- 给已有实例追加/升级/移除加载器。
- Forge/NeoForge、OptiFine、LiteLoader、Cleanroom。
- Fabric API / Quilt API 自动安装。
- 依赖实例隐藏、实例重命名和复制。
- 真实加载器游戏启动验收。
