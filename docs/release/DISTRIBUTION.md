# 平台与分发

状态：已接受的初始方向

日期：2026-10-25

## 构建平台

RMCL 使用 GitHub Actions 构建并分发各平台原生二进制。首批目标为：

| 操作系统 | 初始架构 | 初始便携发布物 |
| --- | --- | --- |
| Windows | x86_64，MSVC | ZIP |
| Linux | x86_64 | tar.gz |
| macOS | x86_64 | `.app` 压缩包 |
| macOS | aarch64 | `.app` 压缩包 |

其他操作系统、架构和安装包格式需要在依赖与真实设备验证后单独加入，不能仅凭 Rust target 可编译就宣称支持。

## CI 与发布边界

- PR 和常规提交运行格式、lint 与不依赖真实账号的测试。
- 发布构建优先在对应操作系统上原生完成，避免把 GPUI、系统库、签名和公证问题集中到交叉编译环境。
- 每个架构单独构建和验证，不能用 x86_64 产物代替 aarch64 验证。
- 发布任务收集所有目标产物，记录版本、commit、目标三元组和 SHA-256；所有必需目标成功后再发布。
- 默认自动化测试不访问真实账号，不依赖外部下载服务的可用性。

## 打包演进

首个阶段以便携包验证运行依赖和资源定位。后续按需增加：

- Windows：Inno Setup 安装包与代码签名。
- macOS：DMG、Developer ID 签名和公证。
- Linux：针对明确支持的发行版增加 DEB、RPM 或 AppImage。

安装包、便携包和包管理器拥有不同的更新边界。自动更新不得只替换一个二进制而破坏 macOS 签名 bundle 或系统包数据库。

## 应用图标

图标源文件和生成物位于 `packaging/icons/`：

- `rmcl.svg`：透明背景的 3D 等距方块，右面为 R/播放三角标记，128px 及以上使用。
- `rmcl-small.svg`：高对比 2D R，16–64px 使用，避免 3D 细节在小尺寸下模糊。
- `rmcl.icns`、`rmcl.ico` 和 `png/` 由 `packaging/icons/generate.sh` 生成。
- 图标为原创资产，不包含 Mojang、HMCL 或其他项目的纹理和模型。

平台接入：

- macOS：`.app` 的 `Info.plist` 使用 `rmcl.icns`；直接运行开发二进制时由 `NSApplication.setApplicationIconImage` 设置 Dock 图标。
- Windows：`rmcl.ico` 需要嵌入可执行文件资源，窗口图标不能代替 exe 图标。
- Linux：PNG 安装到 `hicolor/<size>x<size>/apps/rmcl.png`，运行时同时设置 X11 窗口图标。

## 自动发布

`.github/workflows/release.yml` 负责 GitHub Release：

- 推送 `v*` 标签时触发完整发布；手动触发只构建并保留工作流 artifact，不创建 Release。
- `prepare` job 校验标签版本（去掉前导 `v`）与 `Cargo.toml` 的 `[workspace.package].version` 一致，避免二进制版本和发布版本脱节。
- 四个原生构建 job 并行运行：

| Runner | 产物 |
| --- | --- |
| `ubuntu-24.04` | `RMCL-<version>-linux-x86_64.tar.gz` |
| `windows-latest` | `RMCL-<version>-windows-x86_64.zip` |
| `macos-15-intel` | `RMCL-<version>-macos-x86_64.zip` |
| `macos-15` | `RMCL-<version>-macos-aarch64.zip` |

- Linux 包包含 `rmcl`、`rmcl.desktop` 和 `rmcl.png`；Windows 包包含 `rmcl.exe` 和 `rmcl.png`，exe 通过 `build.rs` 嵌入 `rmcl.ico`；macOS 包是带 `Info.plist` 和 `rmcl.icns` 的 `RMCL.app`。
- 所有构建成功后，发布 job 下载四个 artifact，生成 `SHA256SUMS`，先创建草稿 Release，上传全部资产后再取消草稿。任一平台失败时不会发布半成品。
- Release 资产保留 GitHub 自动生成的源码归档；构建 artifact 保留 7 天。
- 工作流只使用内置 `GITHUB_TOKEN`，发布 job 才申请 `contents: write`；不需要额外 secrets。

macOS 当前只做 ad-hoc 签名，未做 Developer ID 签名和公证；Windows 可执行文件未做 Authenticode 签名。下载后的 macOS 应用可能需要在 Finder 中右键“打开”，Windows 可能显示 SmartScreen 提示。签名和公证在取得凭据后单独加入。

Runner 标签会随 GitHub 镜像退役而变化：`macos-15-intel` 是当前可用的 Intel 镜像，预计 2027 年退役；后续需要同步更新矩阵和本文。

## 发布步骤

1. 确认目标提交的 CI 已通过；Release workflow 负责构建和分发，不替代 lint、测试和真实运行验收。
2. 更新 `Cargo.toml` 的 workspace 版本并提交。
3. 创建与版本一致的标签，例如 `git tag -a v0.1.0 -m "RMCL 0.1.0"`。
4. 推送标签：`git push origin v0.1.0`。
5. 在 Actions 中确认 `Release` workflow 的四个构建 job 全部成功；发布 job 会自动生成 `SHA256SUMS` 并发布草稿。
6. 下载至少一个平台的产物做启动 smoke，再确认 Release 已发布。

Linux 构建使用 Ubuntu 24.04，产物面向较新的 glibc 和 GPUI Kit 所需图形库；当前 tar.gz 不捆绑 WebKitGTK、GTK、Vulkan 等系统依赖。需要覆盖旧发行版时，应增加容器化构建或 AppImage，而不是把 Ubuntu 24.04 产物描述成通用二进制。

## GPUI Kit 平台约束

RMCL 当前已引入 GPUI Kit 0.8.0 / gpui-pre 0.3.9。其文档列出 macOS 15+、Windows 10+，Linux 图形环境需要 Wayland/X11 与可用 Vulkan 驱动，并将 Ubuntu 24.04 作为验证环境。CI 按该依赖列表安装 Linux 开发库；实际最低系统要求和打包依赖仍需逐平台运行验证，不能提前宣称与 HMCL 相同。

参考：[GPUI Kit Installation](https://gpui-kit.com/docs/installation)、[Packaging](https://gpui-kit.com/docs/packaging)、[桌面界面与 GPUI 桥接](../architecture/USER_INTERFACE.md)。
