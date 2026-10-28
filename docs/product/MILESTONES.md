# 里程碑

状态：已接受

日期：2026-10-25

## 推进原则

按可运行、可验证的纵向切片推进。每个切片同时包含必要的核心逻辑、界面、诊断与测试，不先铺完全部页面再补核心，也不预先实现所有加载器。

## M1：可安装并启动 Vanilla

首个工程里程碑包括：

- 离线账号
- Vanilla 版本识别与安装
- 游戏文件自动下载
- Java 检测、选择、下载与管理
- 游戏启动
- stdout、stderr 与退出状态日志
- 游戏文件检查与修复

### 验收标准

1. 用户可以创建并重新加载离线账号，以稳定的玩家名和 UUID 启动游戏。
2. 用户可以复用一个现有 `.minecraft`，安装 Vanilla，并补齐版本 JSON、client JAR、libraries、natives 和 assets 等必要文件。
3. 下载过程展示阶段与进度，支持失败传播、取消和重试；中断的文件不会被当成完整文件使用。
4. RMCL 能选择与游戏及当前平台相容的 Java；缺少合适 Java 时能够进入自动获取流程，也允许显式覆盖。
5. RMCL 能生成平台正确的启动参数，以实例对应的工作目录启动进程，并持续收集日志与退出状态。
6. 修复流程能够发现约定范围内的缺失或损坏文件并重新获取，且重复执行不会无意义地重装全部内容。
7. 核心逻辑具有不依赖图形环境的自动化测试；首个公开可用版本还须通过 Windows、Linux、macOS 的真实运行验证。
8. 日志与诊断信息不泄露账号秘密，失败时提供可定位阶段的错误，而不只是“启动失败”。

具体游戏版本、Java 发行版和真实测试矩阵在 [未决问题](../../notes/bootstrap/open_questions.md) 中确定后补入本文。

### 当前进度（2026-10-26）

已完成启动器壳、离线与 Microsoft 正版账户（设备代码登录、系统钥匙串令牌、删除、官方皮肤头像、皮肤页、PNG/图片直链换肤、恢复默认和本地皮肤库）、已安装版本扫描、实例 manifest 与版本隔离、旧实例显式迁移、版本继承合并、Fabric/Quilt 加载器安装、Forge/NeoForge 现代 processor installer 安装、自定义实例创建、从已有实例创建加载器实例并复制内容、本地 Mod/资源包/光影扫描与导入/删除/启用禁用、Modrinth 搜索/版本/下载/SHA-1 校验/正式版推荐/本地与远程 Mod 元数据索引/版本范围匹配/安装前置预检/必需依赖解析/内容安装任务、服务器收藏与 Java Edition Server List Ping（MOTD、在线人数、延迟、图标和 5 秒自动刷新）、Quick Play/legacy 一键进服、GitHub Release 自动更新（检查、SHA-256、平台包替换、helper 重启）、远程版本目录、安装/快速启动/取消、任务日志、版本修复与删除、实例重命名、单任务互斥、实例级 Java/最大内存/窗口尺寸选择和启动器可见性设置；正版 token 启动真实 Vanilla 已在 macOS ARM64 实际验证。Forge/NeoForge 已在 macOS ARM64 通过真实 installer 和启动到 ModLauncher 初始化的自动化 smoke。authlib-injector、legacy Forge、主菜单/世界加载验收、Java runtime 列表管理界面、实例级任意启动参数、Windows/Linux 真实运行验收、服务器 Bedrock、真实一键进服验收、真实自动更新/重启验收，以及游戏运行时隐藏/恢复的完整真实启动验证仍待完成。CurseForge、可选依赖/冲突求解、更新检查和整合包属于后续里程碑。

## 后续顺序

以下顺序用于控制依赖关系，不表示这些功能已经完成：

1. **M2：账号与主流加载器**：Microsoft、authlib-injector、Fabric、Quilt、Forge、NeoForge。Microsoft、Fabric、Quilt、Forge 和 NeoForge 的现代 installer 已完成；authlib-injector 与 legacy Forge 待完成。
2. **M3：旧版兼容与更多组件**：旧 Forge、OptiFine、LiteLoader、Cleanroom、Legacy Fabric 等。
3. **M4：内容与整合包**：Mod、资源包、整合包检索、下载、导入与导出。
4. **M5：完整产品化**：剩余 HMCL 功能、HMCL 配置导入、完整主题与国际化、自动更新签名/增量更新、安装包。
5. **持续对齐**：根据功能台账补充差异测试，并逐平台验收。

如实现中发现依赖顺序不合理，可以调整后续阶段，但必须在 notes 记录原因并同步更新本文。
