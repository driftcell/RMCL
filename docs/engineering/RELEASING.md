# 分支、PR 与发布

状态：已启用

日期：2026-10-29

## 分支保护

`main` 已配置保护：

- 禁止直接 push、force push 和删除
- 所有修改通过 Pull Request
- Squash merge，只允许这一种合并方式
- 合并后自动删除 head branch
- 要求 conversation resolution 和线性历史
- 要求 CI 检查通过：
  - `CI success`（汇总 Rust 变更检测、lint、测试矩阵；纯文档/网页改动跳过 Rust job 时也保持成功）
- 要求分支与 `main` 保持最新
- 管理员同样受规则约束
- 审批数设为 0，单人维护可以直接合并自己的 PR

功能分支不受删除限制；PR 合并后自动删除。长期分支（如 `release/*`）如需保留，不应作为自动删除的 PR head，或单独配置规则。

## PR 约定

- PR 标题使用 Conventional Commits，例如 `feat(servers): add server browser`
- 按 `.github/pull_request_template.md` 填写目标、验证命令、平台和 release notes 影响
- Squash merge 后 PR 标题成为 `main` 上的 commit，所以标题必须自解释
- 一个 PR 只做一件事；任务目标、验收标准和验证结果写在 PR 描述中
- 合并前必须看到 `CI success` 通过
- 不直接修改或强推 `main`；release PR 也走同一套检查

## release-plz

RMCL 使用单应用发布模型，[release-plz](https://release-plz.dev/) 只管理 `rmcl` 的版本、release PR 和应用 tag：

- `workspace.package.version` 固定为 `0.1.0`，内部 crate 是未发布模块，统一继承该版本；只有 `apps/rmcl/Cargo.toml` 显式保存应用版本。新增内部 crate 不需要发布配置或版本 bootstrap。
- `release-plz.toml` 在 workspace 默认 `release = false`，只对 `rmcl` 设置 `release = true`。内部 crate 不升版、不发布 crates.io、不创建 tag，也不通过 `changelog_include` 逐条汇总提交。
- `.github/workflows/release-plz.yml` 在 push `main` 和手动触发时运行。原生 `release-plz release-pr` 根据 Conventional Commits 更新应用版本、`Cargo.lock` 和基础 `CHANGELOG.md`，打开 `release-plz-*` PR。
- 当前 `0.x.y` 阶段启用 `features_always_increment_minor = true`：包含 `feat` 的 release PR 提升 minor（`0.4.4` → `0.5.0`），只包含 `fix`/`perf`/`refactor` 的 PR 仍提升 patch。
- `release-plz release` 只处理已合并的 release PR；`release_always = false` 确保普通提交不会直接打标签。只有 `rmcl` 设置 `git_only = true`、`git_tag_enable = true` 和 `git_tag_name = "v{{ version }}"`。
- release-plz 只创建应用 tag，不创建 GitHub Release（`git_release_enable = false`）；现有 `release.yml` 负责四平台构建、`SHA256SUMS` 和 Release。

`features_always_increment_minor` 是刻意偏离 Cargo 默认 0.x SemVer 规则的选择：默认 `feat` 只提升 patch，而 RMCL 在 0.x 阶段仍希望新功能获得可感知的 minor 版本。进入 1.x 后该配置与标准 SemVer 的 feature 规则一致。

发布说明以用户价值为中心，不追求自动罗列所有内部 crate 提交。发布负责人根据相邻应用标签之间的 Git 差异，在 release PR 中人工整理和润色功能、修复、兼容性与升级注意事项。相关失败与取舍见 [release-plz 变更检测](../../notes/development/release_plz_change_detection.md)。

### 内置 Token、人工 CI 批准与自动合并

release-plz 使用工作流内置的 `github.token`，不依赖个人 PAT。新 release PR 的作者是 `github-actions[bot]`；已有 PR 的作者不会因认证方式变化而改写。

- 仓库 Settings → Actions → General 必须允许 GitHub Actions 创建和批准 Pull Request。
- 准备 release PR 的 job 具有 `contents: write` 和 `pull-requests: write`；打 tag 并触发打包的 job 具有 `contents: write`、`pull-requests: read` 和 `actions: write`。
- 内置 Token 创建或更新 PR 后，`opened` / `synchronize` / `reopened` 类型的 PR workflow 会进入待批准状态。准备发布时，由有写权限的维护者点击 **Approve workflows to run**，再等待必需的 `CI success`。
- 人工批准是刻意的发布策略：功能持续合入时 release-plz 可以更新 PR，但完整三平台 CI 不在每次更新后自动消耗资源。批准 workflow 只启动检查，不会绕过必需检查。
- 维护者准备发布时，在批准 CI 的同时为 release PR 开启 Squash auto-merge。只有 `CI success` 和其余分支保护条件全部满足，PR 才自动合并；仓库已启用 auto-merge，且只允许 Squash。auto-merge 由维护者手动开启，不由内置 Token 自动发起合并。
- 不通过 `workflow_dispatch` 代替 release PR 的 `pull_request` 检查；手动 dispatch 产生的检查不满足当前分支保护的必需检查语义。

## 发布流程

1. 功能/fix PR 合并到 `main`
2. release-plz 更新或创建 release PR
3. 检查 release PR 中的版本、changelog 和 Cargo.lock
4. 在 release PR 中补充简短的 Highlights/升级注意事项（不要只保留裸 commit 列表）
5. 准备发布时批准待运行的 PR workflow，并开启 Squash auto-merge，等待 release PR 的 `CI success` 通过
6. 满足全部分支保护条件后自动 Squash merge；release-plz release 创建 `v<version>` tag
7. release job 从 release-plz 输出取得本次实际创建的 tag，显式 dispatch `release.yml`：构建四平台产物、生成 `SHA256SUMS`、使用 `CHANGELOG.md` 对应章节作为 Release body，最后发布
8. 下载至少一个平台产物做启动 smoke；自动更新版本还需确认 Release body 和资产名正确

`CHANGELOG.md` 是 release notes 的规范来源，格式为 Keep a Changelog。release-plz 生成的提交分组只是草稿，发布负责人应在合并前将其整理为用户可读的 Highlights 和升级说明，而不是保留内部 commit 清单。后续 release-plz 更新可能重新生成草稿，因此人工润色应在准备合并前进行。`scripts/release-notes.sh` 从 changelog 提取当前版本章节，供 `release.yml` 使用。

## 手动兜底

如果 release-plz 不可用：

1. 新建 `chore/release-v<version>` 分支
2. 手动更新 `apps/rmcl/Cargo.toml` 的应用版本、`Cargo.lock` 和 `CHANGELOG.md`
3. 通过 PR 合并
4. 在 `main` 的绿色提交上创建 annotated tag `v<version>`
5. 推送 tag 并等待 `release.yml`

不要跳过 PR、检查或 changelog 直接推 `main`。标签保护或 release-plz 故障时，优先修复自动化，不长期依赖手工绕过。

## 验证

- `.github/workflows/release-plz.yml`、`release-plz.toml` 和 `CHANGELOG.md` 在本仓库启用
- `scripts/release-notes.sh` 的版本章节提取由发布 workflow 实际执行
- 2026-10-28 使用 release-plz 0.4.2 在临时 worktree 验证：只有 `rmcl` 从 `0.8.1` 计算到 `0.9.0`，内部 crate 全部保持 `0.1.0`，更新范围只有根包 manifest、`Cargo.lock` 和 changelog 草稿
- 单应用模型下的真实 release PR、tag/release 触发和自动更新资产仍需首次发布时验证
