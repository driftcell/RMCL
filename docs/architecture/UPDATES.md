# 自动更新

状态：GitHub Release 检查、SHA-256 校验、平台包下载和退出后替换已实现；签名校验、增量更新和包管理器集成尚未实现

日期：2026-10-28

## 边界

自动更新默认读取 GitHub Release：

- API：`https://api.github.com/repos/driftcell/RMCL/releases/latest`
- `RMCL_UPDATE_API` 可覆盖 API URL，用于镜像或测试
- Release 必须附带 `SHA256SUMS`；缺少校验和时拒绝安装
- 只接受正式版；`draft` 或 `prerelease` 会被忽略
- 只替换 RMCL 自身的便携发布包，不接管系统包管理器或第三方安装器
- 更新计划保留原始重启参数；任一参数不能表示为 UTF-8 时，在修改安装前返回错误，不会丢弃参数值或留下孤立选项

发布包命名与 [平台与分发](../release/DISTRIBUTION.md) 一致：

| 平台 | 资产 |
| --- | --- |
| macOS ARM64 | `RMCL-<version>-macos-aarch64.zip` |
| macOS x86_64 | `RMCL-<version>-macos-x86_64.zip` |
| Windows x86_64 | `RMCL-<version>-windows-x86_64.zip` |
| Linux x86_64 | `RMCL-<version>-linux-x86_64.tar.gz` |

## 检查

- 启动 5 秒后检查一次；之后每 6 小时自动检查
- 设置页提供手动“检查更新”
- 自动检查失败保持静默（离线、GitHub 限流等），手动检查显示错误
- 版本使用 `semver` 比较；当前版本来自 `CARGO_PKG_VERSION`
- 发现新版本后在设置页显示版本、发布说明、发布页和“下载并安装”
- 更新对话框把 GitHub Release body 作为 Markdown 渲染，标题、列表、链接和代码块不再以纯文本标记显示

## 下载与校验

- 下载到配置文件同目录的 `cache/updates/`
- 流式下载并显示已下载字节/百分比
- 下载完成后校验 GitHub Release 的 `SHA256SUMS`
- 校验失败删除临时文件，缓存中的旧包也不会被当作新包
- 校验通过的包可以复用，避免重复下载
- 下载任务只允许在当前没有安装/启动/内容任务时开始

## 安装

应用不会在运行中直接覆盖自身：

1. 将当前可执行文件复制到更新缓存目录，作为一次性的 updater helper
2. 写入 JSON 更新计划（包路径、目标、校验和、父进程 PID、重启参数）
3. helper 以 `--apply-update <plan>` 启动
4. 主应用退出
5. helper 等待父进程结束，重新校验 SHA-256
6. 解压到目标目录内的临时目录
7. 备份旧文件/应用，原子替换或恢复，重启新版本

平台行为：

- **macOS**：目标是从 `RMCL.app/Contents/MacOS/rmcl` 推导出的整个 `RMCL.app`；解压 ZIP 后替换整个 bundle，再用 `open -n` 启动。不会只替换内部二进制而破坏 bundle。
- **Windows**：目标当前 `rmcl.exe`；使用独立 helper 进程等待父进程退出后替换 ZIP 中的 `rmcl.exe`，再启动。
- **Linux**：目标当前 `rmcl` 二进制；解压 tar.gz 后替换并保留可执行权限。

失败时 helper 尽量恢复备份并启动旧版本，日志写入 `cache/updates/update-<pid>.log`。更新计划、下载包和旧版本备份不包含账号令牌或配置秘密。

## 开发构建

- `debug_assertions` 构建默认禁止安装更新，避免把 release 包替换进 `target/debug`
- 检查更新仍可用于调试
- `RMCL_ALLOW_DEV_UPDATE=1` 可临时允许开发构建测试安装；仅用于本机验证

## 限制

- 当前使用 GitHub Release 和 SHA-256，没有 minisign/TUF 等独立签名；HTTPS 和 GitHub 账号是信任边界
- 未做增量/delta 更新，每次下载完整包
- 没有后台静默安装；必须由用户确认下载安装
- 不支持预发布通道、回滚到旧版本或更新失败后的自动降级
- macOS 若使用 Developer ID 公证包，替换整个 `.app` 可保留新包签名；未配置签名时仍是 ad-hoc 签名
- Windows 位于 `Program Files`、Linux 由包管理器安装时可能没有写权限；当前面向便携包
- GitHub 未认证 API 有速率限制；自动检查周期较长，手动检查失败会显示错误
- 本版本之前的发布包不含 updater；从 `v0.3.0` 升级到首个包含 updater 的版本仍需手动下载一次

## 参考

Zed 的自动更新实现（GPL-3.0-or-later）仅用于行为研究：状态机、自动轮询静默失败、下载进度、Windows helper、macOS 整 bundle 替换和重启语义。RMCL 未复制其代码、注释或资源，具体取舍见 [自动更新调研](../../notes/development/auto_updater.md)。

## 验证

- 单元测试覆盖 Release 资产选择、SHA256SUMS 解析、版本比较、ZIP 解压、文件替换与备份清理
- `rmcl-app` 使用本地 axum mock 覆盖 GitHub Release 检查和下载校验
- 2026-10-26 使用真实 GitHub Release API 检查 `0.3.0`：标签、平台资产和 `SHA256SUMS` 解析成功；未执行真实替换
- 真实 Windows/macOS/Linux 自动替换和重启仍需在对应平台用发布包手工验收
