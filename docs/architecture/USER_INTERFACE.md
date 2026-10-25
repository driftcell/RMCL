# 桌面界面与 GPUI 桥接

状态：最小 Vanilla 桌面窗口已实现

日期：2026-10-25

## GPUI Kit 基线

桌面应用使用 `gpui-kit = 0.8.0`，由其统一提供匹配的 `gpui-pre = 0.3.9`、Base、Component 和默认 assets。应用只直接依赖 `gpui-kit` 并导入 `gpui_kit::*`，不单独覆盖 GPUI 包或混用其他快照。

首版启用默认 `component + assets`，不启用 `gpui-fast`、speech、tree-sitter、inspector 或 profiler。没有 mold/LTO 等二进制优化工作。

平台依据：[GPUI Kit Installation](https://gpui-kit.com/docs/installation)。macOS 需要 15+ 和 Xcode Command Line Tools；Windows 需要 Windows 10+、MSVC C++ 工具链、Windows SDK 和 CMake；Ubuntu 24.04 的 CI 安装文档列出的 fontconfig、Wayland、WebKitGTK、XKB/X11、SSL、zstd 和 Vulkan 开发库。Linux 显示窗口还需要图形会话和可用 Vulkan 驱动，headless CI 编译不能代替运行验证。

调研记录：[GPUI Kit 技术验证](../../notes/development/gpui_kit_bootstrap.md)。

## 所有权与执行器

`apps/rmcl` 在窗口根持有 `Rc<Services>`：

- 一个 multi-thread Tokio `Runtime`，驱动 `rmcl-app` 的安装、Java 和游戏任务。
- 一个 `TaskManager`，持有全部后台任务、取消令牌、快照和日志。
- 当前目标 `Platform`，供每个请求克隆使用。

GPUI 不使用 `block_on` 等待核心任务。按钮回调通过 `Runtime::spawn` 调用 `start_install`、`start_launch` 或 `cancel`，并在 GPUI foreground task 中 await 返回的 `JoinHandle`，然后更新 View。传给 Tokio 的数据必须是 owned `Send` 数据，不捕获 GPUI Context 或 Window。

`TaskManager` 的广播接收器直接移入一个持久保存的 GPUI foreground task。该 task 只处理事件和短 Entity 更新，不拥有安装或游戏 future。后台任务不依赖 View 是否正在显示日志。

## 事件恢复与状态一致性

- 按钮启动返回 `TaskId` 后，UI 立即重新读取 `snapshot` 和 `logs`，避免快速任务在命令结果返回前已经完成导致状态缺失。
- `TaskEvent::Snapshot` 驱动当前任务状态。运行中的任务不接受其他 ID 覆盖，终态后可以切换新任务；旧终态不能回写为 Running，Cancelling 不能回退为 Running。
- 每条 `TaskEvent::GameLog` 携带任务内递增行号，View 拒绝重复行并保留最近 256 行；核心任务仍保留最近 1024 行。
- 广播产生 `Lagged` 时不重新订阅，而是再次读取当前 `snapshot` 和 `logs`。恢复依靠快照和日志序号，避免通道残留事件造成状态回退或重复日志。
- 慢订阅者可能丢失中间进度和日志事件，但不会反压游戏进程；可恢复的是当前快照和仍保留的日志，不承诺找回所有早期输出。

## 当前窗口

单一持续存在的窗口提供：

- Minecraft 目录、Vanilla 版本 ID、离线玩家名输入。
- “安装”、“安装并启动”和“取消 / 停止”命令。
- 当前阶段、文件或 native 进度、任务错误、运行/退出状态和日志滚动区域。

默认目录为 `~/.minecraft`，Java store 为 `~/.rmcl/java`，不使用 HMCL 配置。当前示例默认版本为 `26.3`、玩家名 `Steve`，均可编辑。可用以下参数做隔离 smoke：

```text
--minecraft-root <PATH>
--java-store <PATH>
--version <ID>
--player <NAME>
```

`--help` 显示参数说明。首版没有远程版本列表、原生目录选择器、账号或配置持久化。

## 关闭与当前非目标

关闭窗口会释放应用服务和 Tokio runtime；正在停止的应用任务被中止，`GameProcess` 的 `kill_on_drop` 会终止直接游戏子进程。当前不承诺关闭启动器后游戏继续运行，也不管理进程树。

当前不包含 Microsoft 登录、ModLoader、实例管理、修复界面、完整主题、国际化、崩溃分析、日志导出和发布打包。

## 验证

`apps/rmcl/src/state.rs` 的纯 Rust 测试覆盖任务切换、日志隔离、重复/过期输入和有界缓冲，不需要图形环境。GPUI 依赖编译和原生窗口、按钮、真实游戏启动分别验证。
