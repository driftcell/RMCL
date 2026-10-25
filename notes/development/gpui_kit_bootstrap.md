# GPUI Kit 技术验证记录

日期：2026-10-25

## 调研来源

- [GPUI Kit Installation](https://gpui-kit.com/docs/installation)
- [Getting Started](https://gpui-kit.com/docs/getting-started)
- [Task](https://gpui-kit.com/docs/task)
- crates.io：`gpui-kit 0.8.0`、`gpui-component 0.8.0`、`gpui-pre 0.3.9`
- 本地 crate：`gpui-kit-0.8.0/src/lib.rs`、`gpui-pre-0.3.9/src/app.rs`、`platform.rs`

最终决策与当前实现见 [桌面界面与 GPUI 桥接](../../docs/architecture/USER_INTERFACE.md)。

## 已确认约束

- 应用只依赖 `gpui-kit`，由它选择匹配的 GPUI 快照；不另列 `gpui-pre`。
- Rust 1.92+ 是当前依赖图底线，RMCL 实际验证工具为 Rust 1.99.1。
- macOS 15+；当前 smoke 主机为 macOS 27.0.1，已有 Xcode Command Line Tools。
- `WindowOptions` 使用公开字段；标题属于 `TitlebarOptions`，尺寸属于 `window_bounds`。
- GPUI Task 与 Tokio JoinHandle 的 drop 语义不同：前台事件 task 必须保留，Tokio runtime 必须存活并由多线程 worker 驱动。
- `TaskManager::start_*` 内部调用 `tokio::spawn`，不能直接放进没有 reactor 的 GPUI future 中调用；需要在 `Runtime::spawn` 内执行。

## 编译验证

首次以 `gpui-kit 0.8.0` 默认 `component + assets` 完成最小窗口 `cargo check -p rmcl`。随后加入真实安装/启动 View、runtime、事件桥接和纯状态测试，`cargo check -p rmcl --all-targets --locked` 与 `cargo clippy -p rmcl --all-targets --locked -- -D warnings` 通过。

依赖解析固定 `gpui-pre 0.3.9`。没有启用 GPUI Fast、speech、tree-sitter、inspector、profiler、mold 或 LTO。

## 原生 smoke

使用独立临时 `.minecraft` 与 Java store，通过 macOS System Events 的 accessibility 树操作真实 RMCL 窗口。详细结果在完成后补充；该验证不修改用户日常仓库，也不依赖真实账号。
