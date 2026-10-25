# 当前未决问题

日期：2026-10-25

用途：防止把尚未讨论的方案误写为既定决策

问题确定后，应更新对应 docs，并在这里记录指向决策的链接或移除该条目。

## M1 开始前需要确定

- 首个端到端测试选择哪些 Minecraft 版本？现代版本与 legacy 参数格式分别在哪个里程碑纳入强制验收？
- M1 的“自动下载”首版只要求官方来源，还是必须同时交付镜像选择？候选源和故障回退的具体行为是什么？
- Mojang runtime 是否作为首个自动 Java 来源？哪些平台或游戏版本需要额外 Java 发行版？
- 各平台自动下载 Java 的供应、归档格式、校验与许可如何记录？
- `.minecraft` 的实例隔离首版支持哪些模式？RMCL 自己新增的实例配置放在哪里，如何避免污染标准版本 JSON？
- 修复范围中，client JAR、libraries、assets 和版本 JSON 分别采用存在性、大小、哈希还是容器完整性检查？
- 日志在侧栏有独立入口，异常时显示红点；是否需要单独的崩溃报告页、日志级别过滤和诊断包？
- GPUI Kit 已选定 0.8.0 / gpui-pre 0.3.9，页面、配置和隐藏行为见 [桌面界面与 GPUI 桥接](../../docs/architecture/USER_INTERFACE.md) 与 [启动器配置](../../docs/architecture/CONFIGURATION.md)；发布最低系统版本仍需逐平台运行和打包验证。
- 游戏运行时最小化启动器、退出后恢复，在 Windows 和 Linux 上分别是什么窗口语义？macOS 使用 `deminiaturize`，其他平台需要真实运行验收。

## 工程与发布前需要确定

- MIT `LICENSE` 使用哪个版权主体或贡献者表述？
- `rmcl-cli` 是内部诊断工具，还是长期公开支持的第二套接口？
- 配置已使用 `~/.config/rmcl/config.toml`（Windows 为 `%APPDATA%\rmcl\config.toml`），后续 schema 迁移和损坏文件备份策略如何设计？
- GPUI 后台执行器与独立异步运行时如何桥接，哪些 crate 负责具体运行时？
- 发布签名、公证、更新渠道和安装包格式由谁持有凭据并维护？

## 后续功能调研

- 完整 HMCL 功能台账尚未建立，不能把 M1 参考源码清单当成完整功能列表。
- Microsoft 正版登录、令牌刷新、系统凭据存储和官方皮肤头像已实现，见 [正版账号、令牌与皮肤](../../docs/architecture/AUTHENTICATION.md)；自有 Azure 应用注册、披风和 authlib-injector 仍未确定。
- CurseForge、Modrinth、镜像站及其他第三方服务需要分别确认 API、凭据和分发条款。
- HMCL 配置导入的格式范围和失败恢复策略留到对应附加功能阶段。
