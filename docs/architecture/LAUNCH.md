# 启动计划与游戏进程

状态：当前核心行为已实现；真实游戏验证范围见末节

日期：2026-10-25

## 边界与输入

`rmcl-launch` 提供 `LaunchOptions`、`LaunchPlan` 和 `GameProcess`。它消费版本元数据、仓库布局、已探测的 `JavaRuntime`、账号和可选 `InstallPlan`，不自行下载游戏文件或选择 Java。

- `LaunchOptions::from_layout` 建立标准 `.minecraft` 路径。
- `from_install_plan` 只把 `PlannedFileKind::Library` 加入 classpath；`NativeLibrary` 只用于提取，client JAR 和 asset 文件不进入 libraries。
- client JAR 固定位于 classpath 最后。现代显式 native classifier 若已被安装计划归类为普通 `Library`，仍按元数据进入 classpath。
- 生成启动计划时，游戏目录、assets、natives、client JAR 和 libraries 统一转为绝对路径，避免切换工作目录后重复解析相对仓库前缀。
- Java 的最低/最高 major、OS 和架构必须满足 `JavaRequirement`，否则在 spawn 前返回 `IncompatibleJava`。

[应用层 Vanilla 安装与启动](APPLICATION.md) 已组合下载、native 解压、Java ensure 和游戏任务。单独构造 `LaunchPlan` 不证明这些文件已经存在。

## 参数生成

实际命令使用参数数组，顺序为：

1. 启动器支持参数；
2. manifest 的 `arguments.jvm`，或 manifest 缺失时的默认 JVM 参数；
3. 用户追加 JVM 参数；
4. `mainClass`；
5. legacy `minecraftArguments` 和/或 modern `arguments.game`；
6. legacy 自定义分辨率参数；
7. 用户追加游戏参数。

manifest 存在但 `arguments.jvm` 为空列表时保持空列表，不隐式恢复默认参数。modern 参数继续由 `rmcl-minecraft` 按 OS、架构和 features 筛选。legacy 使用独立 tokenizer，支持空白分隔、单双引号、双引号内反引号转义和空参数；未闭合引号或末尾转义会报错，不经过系统 shell。

默认 JVM 参数包含 natives、launcher brand/version 和 classpath，并保留 Windows heap dump 与 Windows 10 OS 属性兼容项。支持参数包含 client JAR、RMI/JNDI 和 log4j 安全属性；非 Windows 平台还设置游戏目录上一级的 `user.home`。

占位符覆盖账号、版本、目录、asset index、classpath、路径分隔符和 launcher 信息。未知 `${...}` 保留原文，不静默替换为空。`demo` 和分辨率通过 feature map 控制 modern ruled arguments；legacy 分辨率由启动层追加。

## 账户与秘密处理

`LaunchAccount` 是启动层唯一消费的账户身份：

- 离线账户通过 `from_offline` 保留玩家名和 profile UUID，并在每次调用时生成新的随机 UUID v4 紧凑 token。
- 正版账户通过 `from_microsoft` 使用已解析的 Minecraft access token、profile UUID 和玩家名。
- 正版和离线账户的 `user_type` 当前均为 `msa`，`user_properties` 为 `{}`。

Microsoft refresh token 和 Minecraft access token 的生命周期由 `rmcl-app` 管理，`rmcl-launch` 不读取配置或系统钥匙串。正版 token 刷新和失败语义见 [正版账号、令牌与皮肤](AUTHENTICATION.md)。

- `command_line()` 是真实执行参数，包含秘密，不得用于诊断输出。
- `diagnostic_command_line()` 将 token 替换为 `<redacted>`。
- `LaunchAccount`、`LaunchPlan` 和 `LogRedactor` 的 `Debug` 输出不会暴露保存的 token。
- stdout/stderr 日志在交给调用方前应用同一 redactor。

调用方自行追加的其他秘密需要另行纳入脱敏策略；当前自动处理的是启动账号的 access token。

## 进程与日志

`GameProcess` 使用 Tokio `Command` 直接启动 Java，不经过 shell。工作目录取自计划，环境变量按传入映射叠加，stdin 连接空设备，stdout/stderr 分别收集。

- 每个进程使用容量 64 的有界日志通道，反压作用于日志读取任务。
- 日志按行交付，保留 stdout/stderr 来源，去除行尾换行，非法 UTF-8 使用替换字符。
- `next_event()` 在全部日志排空后才返回 `GameEvent::Exit`；只有退出事件已经返回后，后续调用才返回 `None`。
- `wait()` 丢弃剩余日志事件并等待最终退出状态。
- `GameExit` 保留退出码、Unix signal 和 success。
- `kill()` 终止直接子进程；调用方仍需通过 `wait()` 或 `next_event()` 等待退出并排空日志。丢弃 `GameProcess` 会请求终止直接子进程。

## 真实验证与当前限制

2026-10-25 在 macOS ARM64、Temurin 25.0.5 上使用官方元数据和下载源进行了独立 smoke：

- Vanilla `26.3` 的 82 个游戏文件和 5147 个 asset objects 全部通过 SHA-1/size 校验；真实进程完成 LWJGL、Cocoa 窗口、OpenGL、资源管理器、纹理 atlas 和 OpenAL 初始化，并在 30 秒观察窗口后由测试主动终止。
- Vanilla `1.7.10` 的 34 个游戏文件、674 个 asset objects 和 3 个 native JAR 完成下载与合并，进程进入真实主类并正确报告退出；其官方 LWJGL 2 native 只有 i386/x86_64，因缺少 ARM64 架构而无法完成启动。这不视为旧版本 ARM64 兼容已经实现。
- 离线账号访问 Mojang/Realms 在线服务会产生 401 或认证错误，与本地 Vanilla 启动是否成功分开判断。
- 2026-10-25 使用真实 Microsoft 账户和正版 access token 在 macOS ARM64 启动 Vanilla，用户确认游戏正常进入运行阶段；正版会话的刷新和启动链路已验证。

当前不包含版本继承和 ModLoader 参数合并、专用 Quick Play 配置、日志文件导出、崩溃分析、进程树管理或 GPUI 编排。Windows 和 Linux 的真实游戏启动仍需分别验收，不能由 CI 编译和脚本进程测试替代。
