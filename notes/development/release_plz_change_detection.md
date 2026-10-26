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
