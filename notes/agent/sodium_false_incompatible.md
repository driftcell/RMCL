# Sodium 安装预检误报不兼容

日期：2026-10-29

状态：已修复；属于依赖预检遗漏冲突版本范围的问题，不是 MCP 连接或 build metadata 比较问题。

## 现象

在已安装 Fabric API `0.165.0+26.3` 的 Fabric 26.3 实例中，让 Agent 安装 Sodium 时，安装预检连续两次提示已有的 Fabric API 与 Sodium 不兼容。Sodium 的 Modrinth 版本依赖列表没有声明这类不兼容，Agent 选择停下来询问，没有强制安装。

## 调查结论

2026-10-29 检查了以下真实远程文件：

- Sodium 0.9.2（Modrinth version `bAZQdGpg`）、0.9.3-alpha.1（`v4PSXean`）和 0.9.3-beta.1（`g6ju16VJ`）的 `fabric.mod.json` 都包含 `"breaks": {"fabric-api": "<0.145.1"}`；对应 Modrinth API 的 `dependencies` 均为空。
- Fabric API 0.165.0+26.3（Modrinth version `EQKgpeKu`）的 JAR 元数据为 `id = "fabric-api"`、`version = "0.165.0+26.3"`。
- `preflight_content_install` 已解析并保存冲突的 `version_requirement`，但生成 `incompatible` 时只检查 Mod ID 是否存在，没有判断版本范围，因此把范围外的新版本也列为冲突。

## 修复

- 本地主 Mod 的 `breaks`/`conflicts` 现在复用 `version_satisfies`，按加载器选择 semver 或 Maven 版本规则；只有已安装版本匹配冲突范围时才阻止安装。
- 无版本范围仍表示所有版本冲突。缺少版本或无法解析版本/范围时保守保留冲突，不把未知状态当成兼容。
- 回归测试使用 Sodium 风格的 `<0.145.1`：Fabric API `0.165.0+26.3` 和边界 `0.145.1+26.3` 不产生冲突，`0.144.0+26.3` 仍产生冲突。
- 当前行为见[远程内容与 Modrinth](../../docs/architecture/REMOTE_CONTENT.md#依赖版本与元数据)。

## 验证

- `cargo test -p rmcl-app --test content preflight_applies_remote_metadata_version_ranges_to_conflicts --locked`
- `cargo nextest run -p rmcl-app -p rmcl-content --locked`（169 passed, 8 skipped）
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked`（484 passed, 9 skipped）
- `cargo test --workspace --doc --locked`
