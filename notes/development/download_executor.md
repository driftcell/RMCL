# 下载执行器实现

日期：2026-10-25

状态：已实现并通过本地与真实 HTTPS 验证

## 设计依据

只读下载行为梳理由 `opencode-go/deepseek-v4.1-flash` 完成，参考 HMCL 固定基线。RMCL 没有复制其系统临时目录、无原子保证的移动、私有镜像协议或取消即成功等行为。

先通过 `refactor(domain): extract shared download primitives` 提取 `Sha1Hash` 和 `DownloadRequest`，避免 `rmcl-net` 依赖安装层。

## 实现选择

- `reqwest + rustls`：HTTP(S) 与 TLS，禁用不需要的默认 features。
- `tokio`：异步网络、文件 IO、定时器和有界通道。
- `futures::buffer_unordered`：全局受控并发。
- `tokio::sync::Semaphore`：独立每主机并发限制。
- `tokio-util::CancellationToken`：协作取消。
- `sha1`：流式计算，不把文件整体载入内存。
- `tempfile`：目标同目录临时文件与校验后发布。
- `fastrand`：非安全用途的重试抖动。
- `axum`：仅用于本地测试服务器。

完整性失败可以重试，但每次重试都重新开始。发布前不修改旧文件；无 SHA-1/size 时也不把存在性当作已验证。

## 尝试与修正

- 首轮 16 个网络测试中，15 个通过。进度测试因容量为 2 的通道无人消费而产生预期背压，测试本身形成等待。改为在下载期间并发收集事件后通过，没有削弱有界通道语义。
- 对照任务计划发现最初只有全局并发限制，随后增加独立每主机 semaphore 和峰值计数测试。
- 去掉了生产代码中的 `unwrap`/`expect`，包括预期 size 和哈希状态的显式处理。
- 实际校验覆盖了旧文件保留、临时清理、404 不重试、5xx 重试、hash 失败重试、批量重复目标和 URL 脱敏。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：83 个测试全部通过。
- `cargo test --workspace --doc --locked`
- `cargo tree -p rmcl-net --edges normal --depth 1 --locked` 确认直接运行时依赖。

真实 HTTPS smoke test 使用仓库外临时程序与 Mojang `26.3` 元数据：

- 下载 asset index `34`，SHA-1/size 验证通过，长度 597035 字节。
- 第二次执行返回 `Skipped`，未重复下载。

真实服务验证不进入默认 CI，mock 测试不依赖外网。

## 当前限制

未实现镜像、ETag、Range 续传、内容寻址缓存、容器检查和解压。独立单文件任务之间没有同目标锁，调用方仍需避免跨批量并发写同一路径。
