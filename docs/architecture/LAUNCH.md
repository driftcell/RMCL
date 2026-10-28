# 启动计划与游戏进程

状态：当前核心行为已实现；真实游戏验证范围见末节

日期：2026-10-28

## 边界与输入

`rmcl-launch` 提供 `LaunchOptions`、`LaunchPlan` 和 `GameProcess`。它消费版本元数据、仓库布局、已探测的 `JavaRuntime`、账号和可选 `InstallPlan`，不自行下载游戏文件或选择 Java。

- `LaunchOptions::from_layout` 建立共享游戏目录的 legacy `.minecraft` 路径；`from_instance` 使用实例 manifest 解析出的隔离模式。
- `from_install_plan` 接受 `InstanceLayout`，只把 `PlannedFileKind::Library` 加入 classpath；`NativeLibrary` 只用于提取，client JAR 和 asset 文件不进入 libraries。
- client JAR 固定位于 classpath 最后。现代显式 native classifier 若已被安装计划归类为普通 `Library`，仍按元数据进入 classpath。
- 生成启动计划时，游戏目录、共享 libraries、assets、natives 和 client JAR 统一转为绝对路径，避免切换工作目录后重复解析相对仓库前缀。
- 旧版 `virtual` / `map_to_resources` index 使用安装阶段展开的 `assets/virtual/<index>` 作为 `--assetsDir`；普通 index 继续使用共享 `assets/`。
- `${game_directory}` 来自实例游戏目录；`${library_directory}` 和 `${libraries_directory}` 来自仓库的共享 `libraries/`，不能从游戏目录推导。
- 启动层消费 `resolve_version` 之后的独立 profile；Fabric/Quilt/Forge/NeoForge 的 mainClass、arguments 和 libraries 已合并，client JAR 按 `jar` 指向的基础版本读取。参见 [版本继承、加载器安装与实例创建](LOADERS.md)。
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
7. 用户追加游戏参数；
8. 服务器目标参数（`--quickPlayMultiplayer` 或 legacy `--server`/`--port`）。

manifest 存在但 `arguments.jvm` 为空列表时保持空列表，不隐式恢复默认参数。modern 参数继续由 `rmcl-minecraft` 按 OS、架构和 features 筛选。legacy 使用独立 tokenizer，支持空白分隔、单双引号、双引号内反引号转义和空参数；未闭合引号或末尾转义会报错，不经过系统 shell。

服务器目标使用 `rmcl_domain::ServerAddress`，由实例 manifest 的基础 `minecraft_version` 判断参数形式：`1.20` / `23w14a` 及以上使用 `--quickPlayMultiplayer`，更早版本使用 `--server` + `--port`。无法识别版本时启动计划返回错误，不静默猜测。加载器实例必须使用基础版本，不能用实例 ID 判断。详见 [服务器列表与一键进服](SERVERS.md)。

默认 JVM 参数包含 natives、launcher brand/version 和 classpath，并保留 Windows heap dump 与 Windows 10 OS 属性兼容项。支持参数包含 client JAR、RMI/JNDI 和 log4j 安全属性；非 Windows 平台还设置游戏目录上一级的 `user.home`。

占位符覆盖账号、版本、目录、asset index、classpath、路径分隔符和 launcher 信息。未知 `${...}` 保留原文，不静默替换为空。`demo` 和分辨率通过 feature map 控制 modern ruled arguments；legacy 分辨率由启动层追加。实际执行路径必须能表示为 UTF-8，否则在生成计划时返回 `NonUtf8Path`，不会用替换字符继续启动。

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

Forge/NeoForge installer processors 已由实例创建流程实现，合并后的加载器参数会进入启动 profile。2026-10-25 在 macOS ARM64 使用真实 Java 启动安装后的 Forge 1.21.1 和 NeoForge 21.1.80 实例，日志进入 ModLauncher 初始化并出现对应加载器标记后主动终止；这验证了安装结果、版本继承和启动参数，但主菜单、世界加载和游戏内行为仍需逐平台验收。当前不包含 legacy Forge installer、任意实例级 JVM/游戏启动参数、日志文件导出、崩溃分析、进程树管理或 GPUI 编排。实例级最大内存和窗口尺寸已进入启动计划；服务器目标已通过 Quick Play/legacy 参数进入启动计划。Fabric/Quilt/Forge/NeoForge 的真实游戏启动仍不能由 CI 编译和脚本进程测试替代。
