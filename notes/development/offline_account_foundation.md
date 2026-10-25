# 离线账号基础实现

日期：2026-10-25

状态：已实现并通过本地验证

## 输入

只读行为梳理由 `opencode-go/deepseek-v4.1-flash` 完成，参考 HMCL 固定基线。实现没有复制 Java 代码或 HMCL 测试数据。

## 取舍

- 核心只拒绝空白用户名，保留原始输入；长度和字符集作为警告信息，不阻止创建。
- 使用 `md-5` 计算公开离线 UUID 算法所需的 MD5，使用 `uuid` 表示 UUID 和生成紧凑形式，不自行实现通用哈希或 UUID 类型。
- `OfflineAccount` 不绑定 JSON、HMCL account ID 或皮肤系统，避免提前决定 RMCL 存储格式。
- `with_uuid` 保留显式 UUID，供未来存储恢复和高级设置使用。
- 离线 access token、启动参数和持久化留给对应启动与存储切片；本实现不宣称已经完成完整登录流程。

## 实现位置

- `crates/rmcl-auth/src/offline.rs`
- `docs/compatibility/OFFLINE_ACCOUNTS.md`

同时将原占位应用迁移到 `apps/rmcl`，根 `Cargo.toml` 成为实际 workspace。其他目标 crate 在对应实现到来时再加入。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci`：8 个测试全部通过。
- `cargo test --workspace --doc --locked`

覆盖独立复算的 Steve/Alex UUID、确定性、原始用户名差异、空白名、显式 UUID、紧凑 UUID 和常规名称警告。
