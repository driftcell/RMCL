# Java 检测、探测与选择实现

日期：2026-10-25

状态：已实现并通过本地验证

## 范围

本阶段只处理已有 Java，不下载 runtime。先通过 `refactor(domain): extract platform primitives` 把 OS/架构/平台类型放入 domain，再新增 `rmcl-java`。

## 实现取舍

- 使用 `which` 处理 PATH，不为 Windows 扩展名和可执行文件查找手写一套逻辑。
- 使用 `java -XshowSettings:properties -version` 获取真实版本与架构，不根据目录名猜测。
- 使用 Tokio process + `try_join!`，任一输出流出错或超限都会提前返回并终止子进程。
- 声明 major 作为最低版本，优先匹配同版本但允许更高版本；未声明时以 Java 8 为最低基线并选择最低可用版本，不默认选最新。
- managed、JAVA_HOME、PATH 候选顺序明确并去重，失败候选与可用候选分别返回。
- Linux x86_64 的 pre-1.13 Vanilla 额外限制最高 Java 8；自动扫描支持禁用路径。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：113 个测试全部通过。
- `cargo test --workspace --doc --locked`

测试覆盖真实脚本进程、属性解析、Java 8/9+/EA major、超时终止、过量输出、失败退出、缺失属性、managed 目录布局和选择优先级。

真实 JRE smoke test 使用 Temurin 25.0.5：`JavaDetector` 从 PATH 找到一个 runtime，探测失败数为 0，识别 major 25、`MacOs`、`aarch64` 和 vendor `Eclipse Adoptium`，并成功按 Java 25 + macOS + aarch64 的要求选中 `/usr/bin/java`。

## 后续兼容修正

首次 Java CI 的 Windows managed-layout 测试失败，因为测试数据在 Windows 仍使用 `java` 而不是 `java.exe`。已改为按平台选择真实文件名，不改变生产逻辑。

HMCL 进一步梳理后，RMCL 又补充了 `os.name` 必需验证、未知 OS/arch 拒绝、中立工作目录、Java 环境变量清理、禁用列表和 Linux x64 旧版上限。声明 major 的既有候选判断从“精确匹配”修正为“最低版本”，自动安装仍需按目标 major 精确选择组件。

## 实例级 Java 覆盖

实例 manifest 增加可选 `java` 路径，保持 schema 1。界面通过 `JavaService::discover` 展示已探测的 managed、JAVA_HOME 和 PATH runtime，选择后只更新当前实例。启动时对显式路径重新 `probe`，不符合游戏版本、平台或架构时返回 `JavaOverrideIncompatible`，不会静默回退到自动选择。该行为避免路径被替换或 runtime 升级后继续使用不兼容 Java，同时不改变自动选择策略。
