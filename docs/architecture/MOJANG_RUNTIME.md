# Mojang Runtime 元数据与组件选择

状态：index 与组件选择已实现；安装执行待后续阶段

日期：2026-10-25

## 官方索引

RMCL 使用官方 Java runtime 产品索引：

`https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`

顶层结构是 `platform -> component -> entries`，不包含额外的 `downloads` 包装层。`gamecore` 与 `minecraft-java-exe` 不属于桌面 Java runtime 组件，选择时忽略。

## 平台键

平台键由明确的目标 OS 与架构决定，不使用启动器宿主架构代替：

| OS / 架构 | 键 |
| --- | --- |
| Windows x86 | `windows-x86` |
| Windows x86_64 | `windows-x64` |
| Windows aarch64 | `windows-arm64` |
| Linux x86 | `linux-i386` |
| Linux x86_64 | `linux` |
| macOS x86_64 | `mac-os` |
| macOS aarch64 | `mac-os-arm64` |

Linux ARM64 不映射到 x86_64 的 `linux`；其他未知架构直接报 `UnsupportedPlatform`。

## 组件与回退

规范组件：

- Java 8：`jre-legacy`
- Java 16：`java-runtime-alpha`
- Java 17：`java-runtime-beta`
- Java 21：`java-runtime-delta`
- Java 25：`java-runtime-epsilon`

组件存在但数组为空不等于有可用 runtime。Java 17 在 ARM64 上可能只有 `java-runtime-gamma` 或 `java-runtime-gamma-snapshot` 可用，因此会按确定顺序回退，不能把静态 beta 映射当成必然可用。

选择 entry 时只接受 `version.name` 解析出的 major **等于**目标 major，再按 `released` 最新优先。不得用“major 大于目标”替代，避免需要 Java 17 时安装 Java 25。

缺失平台、组件或匹配 entry 时返回结构化错误，不安装其他 major。

## 默认 Java 代际

版本 JSON 没有 `javaVersion` 时使用 RMCL 兼容表：

- `>=26.1`：25
- `1.20.5+`：21
- `1.18–1.20.4`：17
- `1.17.x`：16
- `1.13–1.16.x` 及 legacy：8

已有候选的 major 判断是最低版本，自动安装的组件选择才是目标 major 精确匹配。

## 当前边界

本阶段实现 index 模型、平台键、组件回退、entry 选择和 manifest 文件类型模型。真实文件下载、staging、链接、权限、安装 marker 与复用在下一阶段实现。

验证记录见 `notes/development/mojang_runtime_metadata.md`。
