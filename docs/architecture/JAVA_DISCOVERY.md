# Java 检测、探测与选择

状态：当前核心行为已实现

日期：2026-10-25

## 模型与依赖方向

`rmcl-java` 使用 `rmcl-domain` 的 `Architecture` 和 `rmcl-minecraft` 的版本元数据，不依赖 GPUI。

- `JavaRuntime`：实际可执行路径、完整版本、major、架构、可选 vendor 和来源。
- `JavaRequirement`：Mojang component、可选 major 和目标架构。
- `JavaDetector`：发现候选并执行真实探测，返回可用 runtime 与逐候选失败。
- `select_java`：从已验证候选中选择兼容 runtime。

`OperatingSystem`、`Architecture`、`Platform` 已统一放在 `rmcl-domain`，`rmcl-minecraft` 继续 re-export，安装与实例路径保持兼容。

## 候选来源

当前按以下顺序发现并去重：

1. 显式 managed roots：根目录的 `bin/java`、子目录的 `bin/java` 和 `Contents/Home/bin/java`。
2. `JAVA_HOME/bin/java`。
3. PATH 中的 `java`，通过 `which` 解析。

可执行文件必须是实际文件，允许解析符号链接。Windows 使用 `java.exe`，其他平台使用 `java`。

Windows 注册表、更多厂商安装目录和系统包管理器发现尚未实现。

## 真实探测

探测通过 `java -XshowSettings:properties -version` 获取：

- `java.version`
- `os.arch`
- 可选 `java.vendor`

支持 `1.8.0_202` 形式的 Java 8 major，以及 9+ 和早期版本数字前缀。`amd64`、`x64` 等归为 `x86_64`，`arm64` 归为 `aarch64`；未知架构保留但不会猜测位数。

默认每个候选最多运行 5 秒，每路输出最多 64 KiB。超时、输出过量、退出失败、缺少属性和非法版本分别返回结构化错误。错误候选会被记录，不会被加入可用 runtime 列表。

## 选择策略

- `GameVersion.javaVersion.majorVersion` 存在时，要求 major 精确匹配。
- 未声明 major 时，以 Java 8 为 legacy 基线，选择满足要求的最低版本。
- 架构必须匹配目标 Java，而不是只看启动器宿主架构。
- 同一 major 下优先 RMCL managed runtime，然后才是 JAVA_HOME、系统 PATH 和其他自定义来源。
- 显式 Java 覆盖属于实例配置职责，不能通过调整全局排序暗中覆盖。

没有可用候选时，`select_java` 返回 `None`，由上层进入自动下载或展示错误。

## 当前不包含

- Mojang runtime 产品清单与自动下载
- LZMA、runtime link 和托管安装发布
- 启动参数、游戏进程与日志
