# 分支、PR 与发布

状态：已启用

日期：2026-10-28

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

### 必需的 PAT

GitHub 默认 `GITHUB_TOKEN` 创建的 PR 不会触发其他 PR workflow，release PR 无法满足必需检查。因此 release-plz 需要 `RELEASE_PLZ_TOKEN`：

- Fine-grained PAT，仅授权 `driftcell/RMCL`
- 权限：Contents 读写、Pull requests 读写
- 保存为 Actions secret `RELEASE_PLZ_TOKEN`
- 不要提交到仓库或写入日志
- token 过期或轮换时同步更新 secret

未配置 secret 时 workflow 会输出 notice 并跳过 release-plz，不会让普通 CI 失败。配置后首次运行会创建第一个 release PR。

## 发布流程

1. 功能/fix PR 合并到 `main`
2. release-plz 更新或创建 release PR
3. 检查 release PR 中的版本、changelog 和 Cargo.lock
4. 在 release PR 中补充简短的 Highlights/升级注意事项（不要只保留裸 commit 列表）
5. 等待 release PR 的 `CI success` 通过
6. Squash merge release PR；release-plz release 创建 `v<version>` tag
7. tag 触发 `release.yml`：构建四平台产物、生成 `SHA256SUMS`、使用 `CHANGELOG.md` 对应章节作为 Release body，最后发布
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
