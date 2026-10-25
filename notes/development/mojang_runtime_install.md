# Mojang runtime 安装与复用实现

日期：2026-10-25

状态：已实现并通过本地、mock 和真实 runtime 验证

## 范围

在 index/component 选择之后，新增 `MojangRuntimeInstaller`、安装锁、staging、raw 下载、链接与权限、marker 和复用验证。首版只支持 raw，lzma-only 明确报错，不实现 LZMA 解压。

## 关键实现

- index/manifest 的 SHA-1 与 size 都验证，manifest 限制 1 MiB；只有 schema 通过后才处理文件。
- 使用 `rmcl-net` 下载 raw，要求每个 raw 的 SHA-1 和 size。
- 使用全新 staging，所有文件成功后才发布；不先清空旧 runtime。
- 使用 `PathRegistry` 检查重复、大小写冲突和 file/dir 冲突。
- link 目标允许相对 `..`，但规范化后必须在 runtime 根内；目标必须存在且为普通文件或目录。
- Unix 按 manifest 设置权限，macOS 使用 bundle 路径，最后真实 probe。
- marker 最后以 `tempfile + sync + persist` 原子写入；复用时重新验证 marker、所有文件、link 和 probe。

## 尝试与修正

- 初版将 `Directory + Link` 误判为 file/dir 冲突，修正为按实际父条目类型判断，并加入合法目录父级回归测试。
- 为嵌套 map 的 mock 构造器补充 `values_mut().flatten()`，避免把键值 tuple 当 runtime entry。
- 修正 `System::Dos` 中变量遮蔽、Unix 权限类型和 macOS bundle 测试 URL。
- 增加 component 单路径组件校验，防止远程 index 或版本 JSON 通过 component 逃逸目录。
- manifest 下载先检查声明大小，复用时额外要求 probe 版本与 marker 一致。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run -p rmcl-java --locked --profile ci`：38 个 Java 相关测试全部通过。
- mock 覆盖完整安装、marker 复用、损坏文件、缺失 raw、路径逃逸、link 逃逸、raw/manifest hash 不符、probe 失败、macOS bundle 和安装锁。
- 真实 Mojang runtime smoke test：macOS aarch64 安装 Java 21.0.7，识别 `jre.bundle/Contents/Home/bin/java`；第二次安装通过 marker、文件校验和 probe 后直接复用。

真实下载只用于手动 smoke test，不进入默认 CI，也不提交 runtime 二进制。
