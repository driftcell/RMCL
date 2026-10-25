# 版本元数据基础实现

日期：2026-10-25

状态：已实现并通过本地验证

## 范围

本次加入 `rmcl-minecraft`，实现远程版本列表、具体版本 JSON、libraries 基础模型、rules 和 modern 参数筛选。没有实现继承/patch、下载、natives 选择、legacy tokenizer 或进程启动。

## 实现取舍

- 使用 `serde` 与 `serde_json`，不为元数据手写 JSON parser。
- 使用 `regex` 和结构化 `RuleError`，对非法平台表达式显式报错。
- 使用严格的已知字段类型，不复制 HMCL 对错误类型的静默忽略。
- feature 缺失按 `false`，作为 RMCL 的明确规则语义记录。
- 同时保留 modern 与 legacy 参数模型，但不在解析层提前决定启动拼接。
- fixtures 全部为独立构造的最小 JSON，没有复制 HMCL 测试资源。

详细当前行为见 `docs/compatibility/VERSION_METADATA.md`。

## 尝试与修正

第一轮 nextest 运行 26 个测试时，规则测试发现 `x86|x86_64` 对 `x86_64` 的匹配失败。原因是实现只检查 `Regex::find` 返回的第一个匹配，较短分支先命中后被误判为不满足全字符串匹配。

修正为把原始表达式放入 `^(?:...)$` 后调用 `Regex::is_match`，保留 alternation 的完整匹配语义。相同测试随后通过，验证了这次不是修改测试来掩盖问题。

## 最终验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：26 个测试全部通过。
- `cargo test --workspace --doc --locked`

尚未运行真实 Mojang 元数据或真实游戏，因此真实版本兼容矩阵仍未验证。
