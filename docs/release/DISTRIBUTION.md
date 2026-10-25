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

## GPUI Kit 平台约束

2026-10-25 查阅的 GPUI Kit 0.8.0 文档列出 macOS 15+、Windows 10+，Linux 图形环境需要 Wayland/X11 与可用 Vulkan 驱动，并将 Ubuntu 24.04 作为验证环境。RMCL 当前尚未引入 GPUI Kit；实际最低系统要求必须在依赖引入和打包验证时重新确认，不能提前宣称与 HMCL 相同。

参考：[GPUI Kit Installation](https://gpui-kit.com/docs/installation)、[Packaging](https://gpui-kit.com/docs/packaging)。
