# Java 检测、探测与选择实现

日期：2026-10-25

状态：已实现并通过本地验证

## 范围

本阶段只处理已有 Java，不下载 runtime。先通过 `refactor(domain): extract platform primitives` 把 OS/架构/平台类型放入 domain，再新增 `rmcl-java`。

## 实现取舍

- 使用 `which` 处理 PATH，不为 Windows 扩展名和可执行文件查找手写一套逻辑。
- 使用 `java -XshowSettings:properties -version` 获取真实版本与架构，不根据目录名猜测。
- 使用 Tokio process + `try_join!`，任一输出流出错或超限都会提前返回并终止子进程。
- 声明 major 精确匹配；未声明时以 Java 8 为最低基线并选择最低可用版本，不默认选最新。
- managed、JAVA_HOME、PATH 候选顺序明确并去重，失败候选与可用候选分别返回。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：113 个测试全部通过。
- `cargo test --workspace --doc --locked`

测试覆盖真实脚本进程、属性解析、Java 8/9+/EA major、超时终止、过量输出、失败退出、缺失属性、managed 目录布局和选择优先级。

真实 JRE smoke test 使用 Temurin 25.0.5：`JavaDetector` 从 PATH 找到一个 runtime，探测失败数为 0，识别 major 25、`aarch64` 和 vendor `Eclipse Adoptium`，并成功按 Java 25 + aarch64 的要求选中 `/usr/bin/java`。
