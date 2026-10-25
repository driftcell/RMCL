<p align="center">
  <img src="packaging/icons/png/rmcl-512.png" width="144" alt="RMCL icon">
</p>

<h1 align="center">RMCL</h1>

<p align="center">
  A modern Minecraft: Java Edition launcher built with Rust and GPUI Kit.
</p>

<p align="center">
  <a href="https://github.com/driftcell/RMCL/actions/workflows/ci.yml">
    <img src="https://github.com/driftcell/RMCL/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI">
  </a>
</p>

RMCL 是一个独立的 Minecraft Java 版启动器，使用 Rust 和 GPUI Kit 构建原生桌面界面。目标是提供快速、清晰、可维护的 Vanilla 启动体验，并逐步补齐账号、实例和内容管理能力。

## 当前能力

- 安装、快速启动、修复和删除 Vanilla 版本
- 新实例默认版本隔离，旧实例可显式迁移并复制全局内容
- 实例详情页管理 Mod、资源包和光影：本地扫描、导入、删除与启用禁用
- 从 Modrinth 搜索并安装 Mod、资源包和光影，下载带 SHA-1 校验、进度和取消
- Microsoft 正版账户与离线账户，支持账户切换和删除
- 正版刷新令牌存入系统凭据存储，启动时按需刷新
- 从 Mojang 官方纹理生成并缓存正版皮肤头像
- Java 自动获取、探测和选择
- 安装与游戏日志、任务进度、取消和异常退出提示
- 明暗主题与五种强调色，单任务互斥，启动器可见性设置

## 下载

Release 构建由 GitHub Actions 生成，发布在 [Releases](https://github.com/driftcell/RMCL/releases)。

当前提供 macOS（Intel / Apple Silicon）、Windows x86_64 和 Linux x86_64 产物。macOS 未做 Developer ID 签名和公证，Windows 未做 Authenticode 签名。

### macOS 首次打开

当前 macOS 产物使用 ad-hoc 签名，Gatekeeper 可能提示“已损坏”或阻止打开。解压后在终端执行：

```bash
xattr -dr com.apple.quarantine RMCL.app
open RMCL.app
```

这是未公证构建的临时办法。配置 Apple Developer 签名凭据后，发布流程会自动改为 Developer ID 签名和 `notarytool` 公证。

## 从源码运行

需要 stable Rust 工具链和对应平台的 GPUI Kit 系统依赖。

```bash
cargo run -p rmcl
```

平台要求：

- macOS 15+
- Windows 10+
- Linux：Wayland/X11、Vulkan 和 GPUI Kit 文档列出的 GTK/WebKitGTK 依赖

更多构建和打包细节见 [平台与分发](docs/release/DISTRIBUTION.md)。

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
```

## 文档

- [文档索引](docs/INDEX.md)
- [产品范围](docs/product/SCOPE.md)
- [里程碑](docs/product/MILESTONES.md)
- [架构边界](docs/architecture/WORKSPACE.md)
- [桌面界面](docs/architecture/USER_INTERFACE.md)
- [正版账号与皮肤](docs/architecture/AUTHENTICATION.md)
- [开发约定](AGENTS.md)

## 状态

项目处于早期开发阶段。macOS ARM64 已完成真实 Microsoft 登录和 Vanilla 启动验证；Windows 和 Linux 会进行构建与自动化测试，但仍需逐平台运行验收。

## 致谢

感谢 HMCL 在 Minecraft 启动器行为和兼容性方面提供的公开参考。
