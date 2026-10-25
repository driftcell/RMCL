# Rust 工具链配置

日期：2026-10-25

状态：已决定

## 取舍

选择 `stable`，而不是直接把当前本机的 `1.99.1` 固化为项目版本。当前仍处于快速开发阶段，CI 已经使用 stable，暂时没有足够依据把某个小版本声明为 MSRV 或长期发布基线。

`minimal` profile 加 `rustfmt`、`clippy` 覆盖本地与 CI 的基础需求。不添加 `rust-src`，也不预装发布矩阵之外的 targets，避免把未来需求写进默认开发环境。

如果 GPUI、发布签名或可复现构建后来要求固定版本，需要作为单独决策升级，并同步 CI 与文档。

## 落点

- `rust-toolchain.toml`
- `docs/engineering/TOOLCHAIN.md`
