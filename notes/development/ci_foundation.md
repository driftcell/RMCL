# 初始 CI 工作流

日期：2026-10-25

状态：本地验证与首次 GitHub 云端运行均已通过

## 方案

- `lint` 在 Ubuntu 上集中运行格式、Clippy 和 doctests，避免在三个操作系统重复静态检查。
- `test` 在 Windows、Linux、macOS 矩阵中使用 nextest；矩阵和 nextest profile 都不在首个失败时提前取消其余测试。
- nextest 默认并行度已经是 `num-cpus`，workflow 仍显式写出，便于阅读和稳定复现。
- nextest 不运行 doctests，因此单独使用 `cargo test --workspace --doc --locked`。
- 不默认运行所有 features，不自动重试失败测试，不需要真实账号或 secrets。
- 使用锁文件与 `--locked`，防止 CI 静默更新依赖解析结果。

## 参考与工具

- nextest 官方文档确认 `.config/nextest.toml`、CI profile、`num-cpus` 和 `taiki-e/install-action@nextest` 的用法。
- 2026-10-25 查阅 `actions/checkout` latest 为 v7.0.1，workflow 使用主版本 `actions/checkout@v7`。
- 本地 nextest 为用户安装的 `cargo-nextest 0.9.149`。
- 本地 actionlint 为 v1.7.12，下载归档已按发布页 SHA-256 校验。

## 本地验证

- actionlint 检查 `.github/workflows/ci.yml` 通过。
- 与 workflow 对应的格式、Clippy、nextest 和 doctest 命令已用于当前 workspace。
- 当前 26 个 nextest 测试全部通过。

GitHub 仓库 `git@github.com:driftcell/RMCL.git` 已配置为 origin。

首次云端 [CI 运行 40158221319](https://github.com/driftcell/RMCL/actions/runs/40158221319) 已在提交 `f4b07b5eea6cc7a82a45a7c62934a7f5e95a1dd6` 上完成，四个 job 均为 success：静态检查 29 秒、Ubuntu 28 秒、macOS 30 秒、Windows 1 分钟。
