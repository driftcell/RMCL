# Rust 工具链

状态：已接受

日期：2026-10-25

## 当前配置

根目录 `rust-toolchain.toml` 使用：

- `stable` channel
- `minimal` profile
- `rustfmt`
- `clippy`

本地 rustup 与 GitHub Actions 因而使用一致的 stable 工具链和基础组件。当前不配置额外 target；各平台在原生 runner 上使用宿主 target，后续交叉编译需求必须单独评审。

## 版本策略

当前 `stable` 不等同于 MSRV 声明，也不承诺永远使用某个具体小版本。项目最低 Rust 版本需要在确实准备支持该基线时单独确定并验证。

如果未来发布构建需要完全固定的编译器，应将 channel 改为明确版本，同时更新 CI、发布文档和升级流程，不能只修改本机环境。

依赖解析继续由 `Cargo.lock` 与 `--locked` 固定；工具链 channel 和 Cargo 锁文件解决的是不同层面的可复现性。
