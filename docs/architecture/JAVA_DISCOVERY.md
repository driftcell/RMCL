# Java 检测、探测与选择

状态：当前核心行为已实现

日期：2026-10-27

## 模型与依赖方向

`rmcl-java` 使用 `rmcl-domain` 的 `Architecture` 和 `rmcl-minecraft` 的版本元数据，不依赖 GPUI。

- `JavaRuntime`：实际可执行路径、完整版本、major、操作系统、架构、可选 vendor 和来源。
- `JavaRequirement`：Mojang component、最低/可选最高 major、目标操作系统和架构。
- `JavaDetector`：发现候选并执行真实探测，返回可用 runtime 与逐候选失败。
- `select_java`：从已验证候选中选择兼容 runtime。

`OperatingSystem`、`Architecture`、`Platform` 已统一放在 `rmcl-domain`，`rmcl-minecraft` 继续 re-export，安装与实例路径保持兼容。

## 候选来源

当前按以下顺序发现并去重：

1. 显式 managed roots：根目录的 `bin/java`（Windows 为 `bin/java.exe`）、子目录的 `bin/java` 和 `Contents/Home/bin/java`；同时识别 Mojang store 的 `<platform>/<component>/bin/java` 与 `<platform>/<component>/jre.bundle/Contents/Home/bin/java`。
2. `JAVA_HOME/bin/java`（Windows 为 `bin/java.exe`）。
3. PATH 中的 `java`，通过 `which` 解析。

可执行文件必须是实际文件，允许解析符号链接。Windows 使用 `java.exe`，其他平台使用 `java`。Mojang runtime 安装后的可执行文件路径因此是 Windows `bin/java.exe`、macOS `jre.bundle/Contents/Home/bin/java`、Linux `bin/java`。

Windows 注册表、更多厂商安装目录和系统包管理器发现尚未实现。

## 真实探测

探测通过 `java -XshowSettings:properties -version` 获取：

- `java.version`
- `os.name`
- `os.arch`
- 可选 `java.vendor`

支持 `1.8.0_202`、`1.8-internal`、`8u202` 等形式，以及 9+ 和早期版本数字前缀。`amd64`、`x64` 等归为 `x86_64`，`arm64` 归为 `aarch64`。未知 OS 或架构直接拒绝，不回退为宿主值。

探测使用独立临时工作目录，并移除 `JAVA_TOOL_OPTIONS`、`_JAVA_OPTIONS`、`JDK_JAVA_OPTIONS` 和 `JAVA_OPTIONS`，避免外部环境改变输出或注入行为。默认每个候选最多运行 5 秒，每路输出最多 64 KiB。超时、输出过量、退出失败、缺少属性和非法版本分别返回结构化错误。错误候选会被记录，不会被加入可用 runtime 列表。

## 选择策略

- `GameVersion.javaVersion.majorVersion` 是最低版本：优先选择相同 major，也接受满足兼容条件的更高版本，不因“太新”直接拒绝。
- 未声明 major 时，以 Java 8 为 legacy 基线，选择满足要求的最低版本。
- Linux x86_64 上 pre-1.13 Vanilla 额外限制最高 Java 8，避免旧版加载不兼容 native。
- 操作系统与架构必须匹配目标 Java，而不是只看启动器宿主架构。
- 同一 major 下优先 RMCL managed runtime，然后才是 JAVA_HOME、系统 PATH 和其他自定义来源。
- 自动扫描禁用列表只影响自动发现；显式路径仍由独立 probe 验证。
- 实例 manifest 的 `java` 字段提供显式覆盖。启动前用 `probe` 重新读取真实版本、系统和架构，并通过 `JavaRequirement::accepts` 校验；不满足时返回 `JavaOverrideIncompatible`，不会回退到自动选择。
- 实例 Java 选择器通过 `JavaService::discover` 展示 managed、`JAVA_HOME` 和 PATH 候选；也可输入任意可执行文件，通过 `JavaService::probe` 以 `Custom` 来源探测后保存。选择结果只写入当前实例 manifest。

没有可用候选时，`select_java` 返回 `None`，由上层进入自动下载或展示错误。

## 当前不包含

- LZMA 压缩形式的 runtime 文件；raw 安装见 [Mojang runtime 元数据与安装](MOJANG_RUNTIME.md)
- 启动参数、游戏进程与日志
- 实例级内存、窗口和 JVM 参数覆盖；当前只覆盖 Java 可执行文件
