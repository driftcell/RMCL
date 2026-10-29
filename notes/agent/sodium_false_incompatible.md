# Sodium 安装预检误报不兼容（待排查）

日期：2026-10-29

状态：已记录，未修复；属于依赖预检/版本比对问题，不是 MCP 连接问题。

## 现象

在已安装 Fabric API `0.165.0+26.3` 的 Fabric 26.3 实例中，让 Agent 安装 Sodium 时，安装预检连续两次提示：

- 已有的 Fabric API 被标记为与 Sodium 不兼容。
- Sodium 官方发布页没有声明这类不兼容。

## 已知上下文

- 实例：Fabric 26.3。
- 已安装 Fabric API：`0.165.0+26.3`，来自 snapshots（测试版）分支构建。
- Sodium 在 26.3 上可选版本包括 `0.9.2` 正式版、`0.9.3-alpha.1`、`0.9.3-beta.1` 测试版。
- Agent 选择停下来询问，没有强制安装。

## 待查方向

1. 复现 `content_install` 的 `dry_run` 预检，保存完整 `incompatible` 条目、`project_id`、`version_id` 和本地 Mod 元数据。
2. 确认冲突来源是 Sodium 的 `incompatible` 依赖声明，还是本地 Fabric API 的 Mod ID/版本范围匹配误判。
3. 检查 Fabric API snapshots 版本 `0.165.0+26.3` 与 Sodium 依赖声明的版本比较；重点看 `+` 后 build metadata、预发布段和本地 JAR 元数据优先级。
4. 用 `content_install` 的真实文件和 `dry_run` 对照，确认是否有同名/不同版本依赖被错误映射。
5. 修复前不要在 Agent 中强制绕过预检；预检失败应继续要求用户选择版本或修正依赖。
