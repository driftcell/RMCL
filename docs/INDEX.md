# RMCL 文档索引

## 文档层级

- 根目录 `AGENTS.md`：贡献者与自动化 agent 的操作约束。
- `docs/`：已核实事实和已接受决策。目标设计可以记录于此，但必须明确标注为目标而非现状。
- `notes/`：探索过程、备选方案、实验和未决问题，不作为规范来源。
- 源码与测试：当前实际行为。实现尚未跟上路线图时，不得宣称功能已经完成。

## 产品

- [产品范围](product/SCOPE.md)：功能对齐、界面、平台与数据边界。
- [里程碑](product/MILESTONES.md)：纵向交付顺序与首个里程碑验收标准。

## 架构

- [目标工作区](architecture/WORKSPACE.md)：多 crate 边界和依赖方向。
- [执行模型](architecture/EXECUTION_MODEL.md)：任务、下载、安装、启动与修复的核心约束。
- [应用层 Vanilla 安装与启动](architecture/APPLICATION.md)：版本来源、任务、Java、游戏生命周期、日志与取消。
- [Vanilla 安装计划](architecture/INSTALL_PLAN.md)：纯文件计划、native 选择、完整性与冲突语义。
- [下载执行器](architecture/DOWNLOAD_EXECUTOR.md)：并发、重试、取消、完整性与原子发布。
- [ZIP 安全解压](architecture/ARCHIVE_EXTRACTION.md)：staging、条目校验、路径安全与取消。
- [Java 检测、探测与选择](architecture/JAVA_DISCOVERY.md)：真实 runtime 探测与兼容选择。
- [Mojang runtime 元数据与安装](architecture/MOJANG_RUNTIME.md)：组件选择、raw 安装、链接、marker 与复用。
- [启动计划与游戏进程](architecture/LAUNCH.md)：参数、离线身份、脱敏、日志、退出与真实验证范围。
- [桌面界面与 GPUI 桥接](architecture/USER_INTERFACE.md)：GPUI Kit 基线、导航与页面、任务状态、隐藏与恢复、窗口行为。
- [启动器配置](architecture/CONFIGURATION.md)：TOML 路径、schema、离线账户、原子写入与迁移边界。

## 兼容性

- [HMCL 参考基线](compatibility/HMCL_BASELINE.md)：固定参考版本、使用方式和参考边界。
- [离线账号行为](compatibility/OFFLINE_ACCOUNTS.md)：用户名、UUID 与自定义身份语义。
- [版本元数据行为](compatibility/VERSION_METADATA.md)：清单、规则、参数与当前解析边界。
- [Minecraft 仓库布局](compatibility/REPOSITORY_LAYOUT.md)：标准目录、安全相对路径和 natives 隔离。

## 工程

- [开发与文档规范](engineering/CONVENTIONS.md)：注释、命名、测试和文档维护规则。
- [持续集成](engineering/CI.md)：nextest、三平台测试矩阵和本地复现。
- [Rust 工具链](engineering/TOOLCHAIN.md)：stable channel、组件和版本策略。
- [独立实现与许可](engineering/LICENSING.md)：MIT 路线及其工程前提。

## 发布

- [平台与分发](release/DISTRIBUTION.md)：初始构建矩阵、原生构建和打包方向。

## 初始调研记录

- [初始评估](../notes/bootstrap/initial_assessment.md)
- [HMCL 参考源码梳理](../notes/bootstrap/hmcl_reference_review.md)
- [许可路线取舍](../notes/bootstrap/licensing_decision.md)
- [当前未决问题](../notes/bootstrap/open_questions.md)
- [界面重做与配置记录](../notes/development/ui_redesign.md)
