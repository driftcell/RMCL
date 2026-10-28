# release-plz 变更检测与 changelog_include

日期：2026-10-26

状态：已解决

## 背景

`v0.4.0` 发布后，PR #2 `fix(java): clean Windows-only compiler warnings` 合并到 `main`。Release-plz workflow 运行 [40284279767](https://github.com/driftcell/RMCL/actions/runs/40284279767) 结论为 success，但没有创建 `0.4.1` release PR。

当前 `v0.4.0` 是 annotated tag，指向 `c24d8ca`。tag 之后的提交有两个：

- `f4d59bc chore(release): add release-plz and changelog workflow (#1)`
- `97cc10d fix(java): clean Windows-only compiler warnings (#2)`

## 现象

`Prepare release PR` job 的日志只处理了 `rmcl`：

```text
INFO Latest release of package rmcl: tag `v0.4.0` (version 0.4.0)
INFO determining next version for rmcl 0.4.0
INFO rmcl: already up to date
INFO rmcl: no commit matches the `release_commits` regex
release_pr_output: {"prs":[]}
```

本地用 release-plz 0.4.1、完整 clone 和同一配置可以稳定复现；只有误用默认配置（没有读到 `release-plz.toml`）时才会把所有 package 都升级，不能作为有效验证。

## 原因

当时配置在 workspace 级别设置了：

```toml
release = false
```

并且只给 `rmcl` 设置了 `release = true`，同时用 `changelog_include` 列出其他 crate。

release-plz 0.4.1 的 `Project::new` 只保留 `ReleaseMetadataBuilder::get_release_metadata` 返回 `Some` 的 package；`UpdateRequest` 的实现是 `config.release.then(...)`。因此 `release = false` 的 crate 根本不会进入 package 列表，也不会计算 `rmcl-java` 的 diff。

`changelog_include` 只是把已计算的 package commits 追加到主 package 的 `included_commits`。被 `release = false` 排除的 package 没有 diff，`changelog_include` 自然是空操作。最后 `rmcl` 只能看到 `apps/rmcl` 目录内的提交，`fix(java)` 位于 `crates/rmcl-java`，因此被 `release_commits` 过滤掉。

## 决策

保留所有 workspace crate 的 release-enabled 状态，让 release-plz 分析每个 crate 的提交；通过以下方式把“发布物”收敛到 `rmcl`：

- workspace 默认 `changelog_update = false`，只有 `rmcl` 设为 `true`
- workspace 默认 `git_tag_enable = false`，只有 `rmcl` 设为 `true`
- workspace 默认 `git_release_enable = false`，GitHub Release 继续由 `release.yml` 创建
- workspace 级 `git_tag_name = "v{{ version }}"`，让所有 package 都从同一个应用 tag 读取上次发布版本；否则子 crate 会使用默认的 `<package>-v<version>` tag 模式，被当成 initial release 并扫描全部历史
- 保留 `release_commits = "^(feat|fix|perf|refactor)"` 和 `semver_check = false`

配置见 `release-plz.toml`，最终约定见 [分支、PR 与发布](../../docs/engineering/RELEASING.md)。

## 验证

在 `97cc10d` 的临时 clone 中使用候选配置运行 release-plz 0.4.1：

```bash
release-plz update --manifest-path Cargo.toml --allow-dirty
```

结果：

- `rmcl` 和 `rmcl-java` 都计算为 `0.4.0 -> 0.4.1`
- 根 `CHANGELOG.md` 只新增 `fix(java): clean Windows-only compiler warnings (#2)`，没有把 `0.1.0` 以来的历史重复写入
- 其他没有相关提交的 crate 不会被单独发布

实际 release PR 和 tag 仍需在配置修复合并后验证。

## 单应用发布模型（2026-10-28）

`v0.8.1` 之后加入 MCP crate 时，`Prepare release PR` 在 #75、#76 和 #77 的 main push 上失败。代表性日志：[40636799363](https://github.com/driftcell/RMCL/actions/runs/40636799363) 与 [40641770051](https://github.com/driftcell/RMCL/actions/runs/40641770051)。

错误为 `cannot find package "rmcl-mcp-cli" in workspace`，移除独立 bin 后变为 `rmcl-mcp`。原因是所有 package 共用应用标签检测版本，release-plz 试图在 `v0.8.1` 的 workspace 中找到当时尚不存在的 package。PAT 校验和 `Tag merged release` 均正常，没有误发新版本。

曾验证给新 package 临时设置 `git_only = false`、`publish = false` 可以绕过首次基线问题，但每次新增 crate 都要添加并在下一次 tag 后删除 override，而且内部 crate 同步应用版本并不符合 RMCL 的模块边界。

进一步验证发现，不能只把内部 crate 固定为 `0.1.0` 并设置 `release = false`：这样会丢失上文所述提交分析。`release_commits` 又只能用于 workspace，release-plz 0.4.2 拒绝 package 级 override，无法让根包和内部 crate 采用不同的提交过滤规则。

最终决定采用纯 release-plz 的单应用发布模型：

- workspace 继承版本固定为 `0.1.0`，只有 `apps/rmcl` 显式保存应用版本。
- release-plz 只处理 `rmcl` 的版本、release PR 与 tag，内部 crate `release = false`。
- 不使用 `changelog_include` 自动罗列内部提交。发布负责人根据相邻应用标签的 Git 差异，在 release PR 中人工整理用户可读的功能、修复和升级说明。
- 保留 release-plz 原生 `release-pr` / `release` workflow，不增加自定义分支或 changelog 脚本。

曾用 git-cliff 2.14.2 验证仓库级 changelog 能完整覆盖 #75、#76、#77 并保留历史 Highlights，但这会引入额外工具、配置和发布分支编排。项目实际只需要人工润色后的用户发布说明，因此不采用这层自动化。

最终配置又使用 release-plz 0.4.2 在同一 worktree 验证：只计算 `rmcl 0.8.1 -> 0.9.0`，所有内部 crate 均为 `0.1.0`，变更仅限根包 manifest、lockfile 和 changelog，不再出现新 package 基线错误。规范流程见 [分支、PR 与发布](../../docs/engineering/RELEASING.md)。
