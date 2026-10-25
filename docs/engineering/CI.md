# 持续集成

状态：已接受的初始 CI 设计

日期：2026-10-25

## 工作流

`.github/workflows/ci.yml` 在以下情况运行：

- 推送到 `main`
- Pull Request
- 手动触发

工作流只授予 `contents: read` 权限。同一分支或 PR 的新运行会取消旧运行，避免过期结果和无效资源消耗。

## Job 划分

### `lint`

在 Ubuntu 24.04 上执行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --doc --locked
```

nextest 不执行 doctests，因此 doc tests 必须单独保留。

### `test`

`lint` 通过后，在 Ubuntu 24.04、Windows latest、macOS latest 上并行运行：

```bash
cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail
```

平台矩阵设置 `fail-fast: false`，一个平台失败不会取消其他平台。nextest 的 `ci` profile 同样关闭测试级 fail-fast，确保一次运行报告全部失败测试。

`--test-threads num-cpus` 明确按可用并行度并发执行测试。`--no-tests=fail` 防止未来因 target 配置错误而出现“没有测试但 CI 成功”。

## 配置与依赖

- `.config/nextest.toml` 是仓库共享配置；本地默认 profile 输出失败上下文，CI profile 运行全部测试。
- CI 使用 stable Rust、`Cargo.lock` 和 `--locked`，锁文件必须提交。
- Rust 编译产物由 `Swatinem/rust-cache` 缓存，nextest 由 `taiki-e/install-action` 安装。
- 不默认启用 `--all-features`，避免未来把互斥 feature 错当成必须兼容的组合。
- 初始工作流不需要 secrets，不访问真实账号，也不上传发布产物。

## 本地复现

安装 `cargo-nextest` 后，在 workspace 根目录运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --profile ci
cargo test --workspace --doc --locked
```

CI 编译和测试通过不等于图形界面或真实游戏启动验证通过；后者仍按平台单独验收。

参考：[nextest repository configuration](https://nexte.st/docs/configuration/)、[running tests](https://nexte.st/docs/running/)。
