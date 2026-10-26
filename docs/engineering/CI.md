# 持续集成

状态：已启用

日期：2026-10-27

## 工作流

`.github/workflows/ci.yml` 在以下情况运行：

- 推送到 `main`
- Pull Request
- 手动触发

工作流授予 `contents: read` 和 `pull-requests: read` 权限。同一分支或 PR 的新运行会取消旧运行，避免过期结果和无效资源消耗。

## 路径过滤

`Detect Rust changes` job 使用 `dorny/paths-filter` 判断改动是否涉及 Rust 代码或 Web 站点。只有以下路径变化时才运行 `lint` 和 `test`：

- `apps/**`、`crates/**`
- 根 `Cargo.toml`、`Cargo.lock`
- `rust-toolchain*`、`.cargo/**`
- `.config/**`（nextest 配置）
- `.github/scripts/**`
- `.github/workflows/**`

以下路径变化时运行 `web` job：

- `web/**`
- `.github/workflows/ci.yml`
- `.github/workflows/pages.yml`

纯文档、notes、`CHANGELOG.md`、`release-plz.toml` 等改动会跳过所有业务 job；纯 `web/**` 改动会跳过 Rust 编译但运行 Web 检查。跳过的 job 本身不能被分支保护当作稳定成功，因此必需检查只要求最终的 `CI success` gate；不能改成 workflow 级 `paths`/`paths-ignore`，否则整个 workflow 不运行，必需检查会一直 pending。

`workflow_dispatch` 始终运行完整检查。检测 job 自身失败时，`lint` 仍会执行，避免因为过滤逻辑故障而静默跳过必需检查。

## Job 划分

### `changes`

在 Ubuntu 24.04 上运行 `dorny/paths-filter`，输出 `rust` 和 `web` 布尔值。PR 通过 GitHub API 读取变更文件，push 使用 git diff；手动触发直接视为全部变更。它本身不是分支保护的必需检查，但 `lint`、`test`、`web` 和 `ci-success` 通过 `needs` 依赖它。

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

### `web`

在 Ubuntu 24.04 上安装 Bun 1.4.4，然后在 `web/` 目录执行：

```bash
bun install --frozen-lockfile
bun run check
bun run build
```

`check` 是 `tsc --noEmit`；`build` 使用 Bun 打包 `index.html` 到 `web/dist/`。Pages 部署 workflow 复用同样的命令。

### `ci-success`

`main` 分支保护唯一要求的检查。该 job 使用 `if: always()` 始终运行，并依赖 `changes`、`lint`、`test` 和 `web`：

- `changes` 必须是 `success`；检测逻辑失败时 gate 失败。
- `lint`、`test` 和 `web` 允许 `success` 或 `skipped`，其他结果（失败、取消）都使 gate 失败。
- 因此纯文档改动可以跳过全部业务 job，纯 `web/**` 改动可以跳过 Rust 编译，纯 Rust 改动可以跳过 Web 检查，同时始终得到一个稳定的成功检查。
- 单个 job 或矩阵 context 不再被分支保护直接要求，避免 skipped 或未展开的 `Nextest (matrix)` 检查永久阻塞 PR。

## 配置与依赖

- `.config/nextest.toml` 是仓库共享配置；本地默认 profile 输出失败上下文，CI profile 运行全部测试。
- CI 使用 stable Rust、`Cargo.lock` 和 `--locked`，锁文件必须提交。
- Ubuntu job 在 Rust 安装前通过 `.github/scripts/install-gpui-deps.sh` 安装 GPUI Kit 0.8.0 文档要求的 fontconfig、Wayland、WebKitGTK、XKB/X11、SSL、zstd 和 Vulkan 开发库；不启用 ALSA/speech 等可选 feature。
- 安装脚本先把 `azure.archive.ubuntu.com` 替换为 `mirrors.edge.kernel.org`，并关闭 recommends，避免 GitHub 托管 runner 落到慢速 apt 节点。可用 `UBUNTU_MIRROR` 覆盖镜像地址。
- Rust 编译产物由 `Swatinem/rust-cache` 缓存，nextest 由 `taiki-e/install-action` 安装。
- Web 使用 `web/bun.lock`、Bun 1.4.4 和 `--frozen-lockfile`；`bun install` 在 CI 中只在 `web/` 目录执行。
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

Web 站点在 `web/` 目录运行：

```bash
bun install --frozen-lockfile
bun run check
bun run build
```

CI 编译和测试通过不等于图形界面或真实游戏启动验证通过；后者仍按平台单独验收。

## 发布工作流

标签 `v*` 触发的 `.github/workflows/release.yml` 负责多平台发布构建，包含版本一致性校验、四平台原生构建、草稿 Release、`SHA256SUMS` 和 `CHANGELOG.md` 提取的 release notes；不会替代 CI 测试。发布流程、产物命名和签名边界见 [平台与分发](../release/DISTRIBUTION.md)。

release-plz 在 `main` 上准备 release PR，并在合并后创建 tag；它需要 `RELEASE_PLZ_TOKEN` 才能触发 release workflow。分支保护和 PR 流程见 [分支、PR 与发布](RELEASING.md)。

## 首次云端验证

2026-10-25，提交 `f4b07b5eea6cc7a82a45a7c62934a7f5e95a1dd6` 的 [CI 运行 40158221319](https://github.com/driftcell/RMCL/actions/runs/40158221319) 已完成且结论为 success。静态检查、Ubuntu、macOS 和 Windows 四个 job 全部通过。

参考：[nextest repository configuration](https://nexte.st/docs/configuration/)、[running tests](https://nexte.st/docs/running/)。
