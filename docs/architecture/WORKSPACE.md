# 目标工作区

状态：已接受的目标架构

日期：2026-10-25

## 当前状态

RMCL 已是 Cargo workspace：`apps/rmcl` 提供最小 GPUI Vanilla 安装与启动窗口；`rmcl-app` 提供 headless Vanilla 安装、稳定任务 ID、快照、互斥、取消、Java ensure、游戏生命周期和有界日志；`rmcl-domain` 提供共享平台类型、校验、下载请求与提取计划；`rmcl-auth` 提供离线账号；`rmcl-minecraft` 提供版本元数据与规则；`rmcl-instance` 提供安全仓库布局；`rmcl-install` 提供纯 Vanilla 安装计划；`rmcl-net` 提供并行下载执行；`rmcl-archive` 提供安全 ZIP 解压与合并；`rmcl-java` 提供 Java 检测、探测、选择、Mojang runtime 安装与复用；`rmcl-launch` 提供启动计划、真实进程、日志脱敏和退出状态。其他边界按实现进度加入，本文件其余部分描述目标结构。

## 目标结构

```text
RMCL/
├── apps/
│   ├── rmcl/                 # GPUI Kit 桌面程序
│   └── rmcl-cli/             # 无界面诊断与核心流程入口
├── crates/
│   ├── rmcl-domain/          # 基础领域类型、ID、平台与错误模型
│   ├── rmcl-platform/        # 平台路径、系统集成与安全存储
│   ├── rmcl-net/             # HTTP、候选源、缓存、下载与校验
│   ├── rmcl-minecraft/       # 清单、继承、规则与资源元数据
│   ├── rmcl-java/            # Java 检测、选择、下载与管理
│   ├── rmcl-auth/            # 离线、Microsoft、authlib-injector
│   ├── rmcl-install/         # Vanilla 与 ModLoader 安装编排
│   ├── rmcl-launch/          # 启动计划、进程、日志、退出与崩溃处理
│   ├── rmcl-instance/        # 实例、版本隔离与实例配置
│   ├── rmcl-content/         # Mod、资源包与整合包生态
│   ├── rmcl-store/           # 配置持久化、迁移与原子写入
│   └── rmcl-app/             # 用例、任务、进度与事件编排
├── xtask/                    # 开发和打包自动化
├── packaging/                # 平台打包资源
└── tests/                    # 跨 crate fixtures 与兼容性测试
```

这是逻辑边界，不要求一次性创建所有目录和空 crate。实现到对应能力时再拆分，并优先避免小型 crate 之间出现无意义的循环依赖。

## 依赖方向

```text
apps/rmcl ─────┐
               ├──> rmcl-app ───> 领域服务 crates
apps/rmcl-cli ─┘
```

- `gpui-kit` 仅属于桌面应用及其 UI 组件。
- 核心 crate 不出现 `Entity`、`Window`、`Context` 等 GPUI 类型。
- `rmcl-minecraft` 的解析与计划生成尽量保持纯计算，不自行弹出 UI 或决定页面流程。
- `rmcl-app` 持有应用级任务和状态；View 通过命令与只读快照交互。
- `rmcl-cli` 与 GUI 复用同一核心能力，用于诊断、自动化测试和不依赖显示服务器的验证。

## GPUI 依赖约束

当前使用 GPUI Kit 0.8.0 和匹配的 gpui-pre 0.3.9，在 workspace 统一管理版本并提交锁文件。应用通过 `gpui_kit::*` 使用与其匹配的 GPUI 快照，不另行混用不受该版本支持的 GPUI 包。GPUI Kit 升级单独进行并记录验证结果，桥接与窗口行为见 [桌面界面](USER_INTERFACE.md)。
