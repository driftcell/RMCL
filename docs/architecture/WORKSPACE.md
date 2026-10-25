# 目标工作区

状态：已接受的目标架构

日期：2026-10-25

## 当前状态

RMCL 已是 Cargo workspace：占位应用位于 `apps/rmcl`；`rmcl-domain` 提供共享校验和下载请求；`rmcl-auth` 提供离线账号；`rmcl-minecraft` 提供版本元数据与规则；`rmcl-instance` 提供安全仓库布局；`rmcl-install` 提供纯 Vanilla 安装计划。其他边界按实现进度加入，本文件其余部分描述目标结构。

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
│   ├── rmcl-minecraft/       # 清单、继承、规则、资源和启动计划
│   ├── rmcl-java/            # Java 检测、选择、下载与管理
│   ├── rmcl-auth/            # 离线、Microsoft、authlib-injector
│   ├── rmcl-install/         # Vanilla 与 ModLoader 安装编排
│   ├── rmcl-launch/          # 进程、日志、退出与崩溃处理
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

引入 GPUI Kit 时，在 workspace 统一管理版本并提交锁文件。应用通过 `gpui_kit::*` 使用与其匹配的 GPUI 快照，不另行混用不受该版本支持的 GPUI 包。GPUI Kit 升级单独进行并记录验证结果。
