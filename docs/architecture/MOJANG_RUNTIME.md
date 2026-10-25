# Mojang Runtime 元数据与组件选择

状态：index、组件选择、raw 安装与复用已实现

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

## 安装执行

`MojangRuntimeInstaller` 按以下顺序执行：

1. 获取安装锁并尝试有效复用。
2. 获取 index 和 manifest，验证 SHA-1、size、schema 和官方 URL 策略。
3. 在最终目录所在文件系统创建全新 staging。
4. 校验所有条目路径、重复/冲突和 link 目标，生成 raw 下载请求。
5. 使用 `rmcl-net` 并发下载并校验 raw SHA-1/size。
6. 创建目录、合法相对 link 和文件权限，再审计完整文件树。
7. 对最终 Java 可执行文件做真实 probe，验证 OS、架构和版本。
8. 发布 staging，最后以临时文件 + sync + persist 写入 marker。

`raw` 是首版唯一支持的压缩形式；lzma-only 条目返回 `MissingRawDownload`，不会静默跳过或留下不完整 runtime。远程 manifest 与 index 都限制为 1 MiB，并要求 manifest 的 SHA-1 和 size。

## 链接与权限

- 普通条目只允许相对路径，拒绝绝对路径、盘符、反斜杠、控制字符和重复/冲突路径。
- link 目标允许 `..`，但必须以 link 父目录为基准规范化后仍位于 runtime 根内。
- 目标必须存在且为普通文件或目录；Windows 无符号链接权限时返回明确错误，不复制目标文件代替。
- Unix 文件按 manifest `executable` 设置 `0o755` 或 `0o644`，目录为 `0o755`；Windows 不应用 POSIX 执行位。
- macOS 使用 `jre.bundle/Contents/Home/bin/java`，其他平台使用 `bin/java`。

## Marker 与复用

`<platform>/<component>.json` 使用 schema 版本 1，记录来源、平台、component、目标 major、manifest 信息、可执行文件、实际 probe 版本/vendor 和所有文件类型、SHA-1、size、link target。

复用必须同时满足：

- marker 可解析且 schema、平台、component 和 major 匹配。
- 所有文件存在且类型正确，SHA-1/size 匹配，link 目标仍合法。
- 可执行文件存在，真实 probe 成功，版本与 marker 一致并满足当前 requirement。

目录存在或 marker 存在本身都不算已安装。损坏或来源不明的 runtime 返回 `ExistingRuntimeInvalid`，不会被静默覆盖或当成有效 runtime。

同一组件使用 `.locks` 下的 create-new 锁防止并发安装；锁在任务结束或失败后释放。锁文件异常残留时不会自动删除，以免误删活跃安装。

## 真实验证

2026-10-25 在当前 macOS aarch64 环境完成 Java 21.0.7 官方 runtime 安装，识别 `jre.bundle/Contents/Home/bin/java`，第二次安装通过 marker、文件校验和 probe 后直接复用。详见 `notes/development/mojang_runtime_install.md`。
