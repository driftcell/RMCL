# 自动更新调研与取舍

日期：2026-10-26

## 参考

- Zed `crates/auto_update` 和 `crates/auto_update_helper`：GPL-3.0-or-later。仅阅读行为与架构，不复制代码、注释或资源。
- 采用的行为：状态机（Idle/Checking/Downloading/Installing/Updated/Errored）、自动轮询、自动检查失败静默、下载百分比回调、Windows 独立 helper 等待父进程退出、macOS 替换整个 app bundle、重启路径。
- RMCL 不采用 Zed 的 DMG/安装器/rsync 流程：RMCL 发布的是便携 ZIP/tar.gz，使用 ZIP 安全解压和文件/目录替换。

## 决策

- 默认源固定为 `driftcell/RMCL` 的 GitHub `releases/latest`；`RMCL_UPDATE_API` 允许镜像/测试覆盖。
- 只接受正式版，必须有 `SHA256SUMS`。
- helper 复用当前可执行文件而不是新增独立二进制；更新计划通过 JSON 文件传递。
- 先提示再安装，不静默替换。
- 开发构建默认禁止安装。

## 依赖许可

- `semver`：MIT OR Apache-2.0
- `mvn_version`：MIT
- `sha2`：MIT OR Apache-2.0
- `flate2`：MIT OR Apache-2.0
- `tar`：MIT OR Apache-2.0
- `sysinfo`：MIT
- `reqwest`、`serde`、`tokio` 等已有依赖沿用其原有许可和 Cargo 元数据。

## 未决

- 是否引入 minisign/TUF 等独立签名；当前信任 HTTPS + GitHub + SHA-256。
- 是否支持预发布通道、增量更新和回滚。
- 包管理器安装（deb/rpm/AppImage/安装器）的更新边界。
- Windows 安装到 `Program Files` 时的 UAC 方案。
- macOS 公证/签名包替换后的 Gatekeeper 行为需要真实发布包验证。
