# 平台基础类型整理

日期：2026-10-25

状态：已决定

## 原因

`rmcl-java` 需要操作系统和架构类型，但不应为了这些基础类型依赖整个 Minecraft 元数据层。因此将 `OperatingSystem`、`Architecture`、`Platform` 从 `rmcl-minecraft` 移到 `rmcl-domain`。

`rmcl-minecraft` 继续 re-export 这些类型，现有安装计划和测试 API 保持不变。`rmcl-instance` 改为直接使用 domain 类型，进一步减少不必要的上层依赖。

## 行为边界

不修改 manifest OS 名称、架构别名、指针宽度或 natives 目录命名。移动后仍通过完整 workspace 测试验证规则与布局行为。
