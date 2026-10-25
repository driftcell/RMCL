# 初始评估

日期：2026-10-25

性质：初始现状、取舍与决策落点

## 当前现状

本次开始时，RMCL 只有：

- `Cargo.toml`：package 名 `RMCL`，版本 `0.1.0`，Rust 2024 edition，无依赖。
- `src/main.rs`：打印 `Hello, world!` 的占位入口。
- `.gitignore`：忽略 `/target`。
- 一个尚未产生提交的 git 仓库。

本机观察到 `rustc 1.99.1` 与 `cargo 1.99.1`。这只是开发环境事实，不是 RMCL 的 MSRV 决策。

HMCL 位于相邻目录，工作区在检查时没有未提交修改，当前提交已固定为参考基线。

## 已做的调研

- 阅读 GPUI Kit 0.8.0 的安装、Task、Packaging 和 Auto Update 文档。
- 对 HMCL 当前源码进行只读静态梳理，覆盖 M1 的主要核心路径和测试位置。
- 核对 HMCL 当前 README 中的 GPLv3 与附加条款说明。
- 未修改 HMCL，未运行 HMCL 测试，未开始 RMCL 功能实现。

## 主要取舍

### 独立重写，而不是逐行移植

HMCL 的核心与 UI 调度存在 JavaFX 耦合，机械映射其模块会把旧边界带入 Rust。RMCL 采用 headless 核心、应用任务层和 GPUI 桌面前端的分层。

### 纵向切片，而不是 UI 先行

先打通离线 Vanilla 的安装与启动，让清单、下载、Java、启动和修复共享一条真实链路，再逐步扩展账号与加载器。

### 渐进拆分，而不是预设大量空 crate

目标 crate 边界先记录在 docs，实际拆分和实现同步发生，避免在领域模型尚未验证时固化过多包边界。

### 应用持有长任务

GPUI Task 的所有权语义不适合让某个页面独占下载或游戏进程。核心任务由应用层管理，View 只订阅状态。

### MIT 与独立实现绑定

选择 MIT 原创实现路线，同时设置禁止复制或贴身翻译 HMCL 代码与资源的约束。详见许可记录；不把“架构不同”本身当成许可结论。

## 最终落点

- 产品与里程碑：`docs/product/`
- 目标架构：`docs/architecture/`
- HMCL 基线：`docs/compatibility/HMCL_BASELINE.md`
- 工程与许可：`docs/engineering/`
- 平台分发：`docs/release/DISTRIBUTION.md`
- agent 协作和提交规则：`AGENTS.md`

后续探索任务按用户要求委派给 `opencode-go/deepseek-v4.1-flash`；主 agent 负责实现与最终验证。每个完成且验证通过的任务使用 Conventional Commits 提交。
