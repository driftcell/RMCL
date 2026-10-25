# Mojang runtime 元数据选择

日期：2026-10-25

状态：已实现并通过本地与实时 index 验证

## 范围

本阶段只处理官方 index、平台键、组件回退、entry 选择和 manifest 文件类型模型，不执行 runtime 安装。

## 关键取舍

- 使用明确的目标 OS/arch 映射，不复制 HMCL 中可能使用宿主 arch 的歧义。
- Java 17 允许 beta -> gamma -> gamma-snapshot 回退，解决 ARM64 beta 空数组。
- entry 只接受目标 major 精确匹配，再按 released 选择最新，避免安装更高 major。
- 默认 Java 代际使用显式边界比较，覆盖 `1.20.1` 等 `1.18–1.20.4` 版本。
- manifest 同时保留 raw/lzma、directory/link/executable 结构；LZMA 是否实现由后续安装阶段决定。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run -p rmcl-java --locked --profile ci`：21 个 Java 相关测试全部通过。

实时官方 index smoke test：

- macOS ARM64 Java 17 选中 `17.0.15`，manifest size 97689。
- macOS ARM64 Java 25 选中 `25.0.1`，manifest size 103399。
- Linux x64 Java 21 选中 `21.0.7`，manifest size 82843。

只读取 index，没有下载 runtime 文件。
