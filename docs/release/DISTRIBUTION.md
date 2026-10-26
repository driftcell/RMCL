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

- Linux 包包含 `rmcl`、`rmcl.desktop` 和 `rmcl.png`；Windows 包包含 `rmcl.exe` 和 `rmcl.png`，exe 通过 `build.rs` 嵌入 `rmcl.ico`，release 构建使用 Windows GUI subsystem，不附带控制台窗口；macOS 包是带 `Info.plist` 和 `rmcl.icns` 的 `RMCL.app`。
- 所有构建成功后，发布 job 下载四个 artifact，生成 `SHA256SUMS`，先创建草稿 Release，上传全部资产后再取消草稿。任一平台失败时不会发布半成品。
- Release 资产保留 GitHub 自动生成的源码归档；构建 artifact 保留 7 天。
- 工作流只使用内置 `GITHUB_TOKEN`，发布 job 才申请 `contents: write`；不需要额外 secrets。
- `SHA256SUMS` 同时供手动下载校验和 RMCL 自动更新使用；缺少该资产的 Release 不会被自动更新接受。

## 自动更新

RMCL 默认从本仓库 GitHub Release 检查正式版更新；详细行为和平台替换流程见 [自动更新](../architecture/UPDATES.md)。

- 更新包使用完整发布包，不做增量更新。
- macOS 替换整个 `RMCL.app`，Windows 替换 `rmcl.exe`，Linux 替换 `rmcl` 二进制。
- 更新 helper 使用当前可执行文件的临时副本，等待主进程退出后替换并重启；不使用独立安装器。
- 下载必须通过 `SHA256SUMS` 校验；当前没有额外 minisign/TUF 签名。
- 便携包可以直接替换；系统包管理器、Windows `Program Files` 和需要提权的安装位置不在自动更新承诺内。
- 未配置 Developer ID 时 macOS 仍是 ad-hoc 签名；自动更新不会绕过 Gatekeeper 或系统权限。

macOS 默认使用 ad-hoc 签名，未做 Developer ID 签名和公证时，从浏览器下载的 `.app` 会被 Gatekeeper 隔离。临时打开方式：

```bash
xattr -dr com.apple.quarantine RMCL.app
open RMCL.app
```

这要求用户信任构建来源；不能把 ad-hoc 签名描述为可分发的正式签名。Windows 可执行文件未做 Authenticode 签名，可能显示 SmartScreen 提示。

配置以下仓库 secrets 后，macOS job 会自动改用 Developer ID 签名、hardened runtime、时间戳、`notarytool` 公证和 stapling：

| Secret | 内容 |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | Developer ID Application 证书的 base64 编码 `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 密码 |
| `APPLE_SIGNING_IDENTITY` | 例如 `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple Developer 账户邮箱 |
| `APPLE_APP_SPECIFIC_PASSWORD` | Apple ID app-specific password |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

缺少任一 secret 时 job 会打印 warning 并回退到 ad-hoc 签名；签名导入失败或公证失败会直接让 job 失败，不会静默发布未签名包。临时 keychain 在 job 结束时删除。正式签名需要加入 Apple Developer Program，凭据由仓库维护者持有，不能提交到仓库。

完整的证书导出、secrets 配置和产物验证步骤见 [macOS 签名与公证](MACOS_SIGNING.md)。

Runner 标签会随 GitHub 镜像退役而变化：`macos-15-intel` 是当前可用的 Intel 镜像，预计 2027 年退役；后续需要同步更新矩阵和本文。

## 发布步骤

1. 确认目标提交的 CI 已通过；Release workflow 负责构建和分发，不替代 lint、测试和真实运行验收。
2. 功能/fix PR 合并后，由 release-plz 创建 release PR；检查版本、`CHANGELOG.md` 和 `Cargo.lock`，补充用户可见的 Highlights。
3. 在 release PR 的四个必需检查通过后 Squash merge；release-plz 创建与版本一致的 `v<version>` 标签。
4. 标签触发 `release.yml`；发布 job 从 `CHANGELOG.md` 提取当前版本作为 Release body，生成 `SHA256SUMS` 并发布。
5. 在 Actions 中确认 `Release` workflow 的四个构建 job 全部成功。
6. 下载至少一个平台的产物做启动 smoke，再确认 Release 已发布。

release-plz、`RELEASE_PLZ_TOKEN` 和手动兜底流程见 [分支、PR 与发布](../engineering/RELEASING.md)。

Linux 构建使用 Ubuntu 24.04，产物面向较新的 glibc 和 GPUI Kit 所需图形库；当前 tar.gz 不捆绑 WebKitGTK、GTK、Vulkan 等系统依赖。需要覆盖旧发行版时，应增加容器化构建或 AppImage，而不是把 Ubuntu 24.04 产物描述成通用二进制。

## GPUI Kit 平台约束

RMCL 当前已引入 GPUI Kit 0.8.0 / gpui-pre 0.3.9。其文档列出 macOS 15+、Windows 10+，Linux 图形环境需要 Wayland/X11 与可用 Vulkan 驱动，并将 Ubuntu 24.04 作为验证环境。CI 按该依赖列表安装 Linux 开发库；实际最低系统要求和打包依赖仍需逐平台运行验证，不能提前宣称与 HMCL 相同。

参考：[GPUI Kit Installation](https://gpui-kit.com/docs/installation)、[Packaging](https://gpui-kit.com/docs/packaging)、[桌面界面与 GPUI 桥接](../architecture/USER_INTERFACE.md)。
