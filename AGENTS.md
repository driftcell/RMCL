# RMCL 开发约定

RMCL 是一个独立的 Rust Minecraft 启动器，采用自己的架构与实现路线，目标界面使用 GPUI Kit。

## 开始工作前

1. 阅读 `docs/INDEX.md`，再阅读任务相关的 docs 与 notes。
2. 区分当前已实现行为、已接受的未来设计和探索中的想法；不要把目标架构描述成现状。
3. 检查工作区已有修改，保留用户和其他贡献者的工作。

## 实现与验证

- 主 agent 亲自负责代码实现、针对性修改、文档定稿和最终验证。
- 为修改已知文件而读取其上下文，以及修改后的验证，不需要另行委派。

## 开发路线

- 以可验收的纵向功能切片推进，不采用一次性全量重写。
- 当前首个里程碑：离线账号、Vanilla、自动下载、Java 管理、启动、日志和修复。
- 核心逻辑不得依赖 GPUI；桌面 UI 只负责交互和展示。
- 长生命周期任务由应用层持有，不能由某个 View 或页面临时持有。
- 多 crate 结构按真实模块边界逐步拆分，不预先创建空壳 crate。
- 优先复用现有 `.minecraft`，RMCL 自有配置独立存放。

详细决策见 `docs/product/`、`docs/architecture/` 和 `docs/compatibility/`。

## GPUI

GPUI 的使用参考 <https://gpui-kit.com/llms.txt>。

涉及 GPUI 部分的开发必看：

- <https://gpui-kit.com/docs/design-guides.md>
- <https://gpui-kit.com/docs/coding-guides.md>

## 文档纪律

- `docs/` 只记录已核实事实和最终决策。文件名使用全大写，目录使用全小写 snake_case，例如 `docs/architecture/EXECUTION_MODEL.md`。
- `notes/` 记录调研、假设、尝试、取舍和阶段性结论。目录与文件名均使用全小写 snake_case，例如 `notes/bootstrap/initial_assessment.md`。
- notes 不是规范来源。形成最终决策时，必须同步更新对应 docs，并保留指向调研 notes 的链接。
- 新事实应更新已有文件，避免创建内容重复的记录。未决定的问题留在 notes，不写成 docs 中的事实。
- 外部行为引用尽量使用固定 commit 的链接；易过期的信息注明日期和来源。

## 代码与注释

- 使用惯用 Rust，保持模块边界、错误信息和平台行为明确。
- 优先评估成熟、维护活跃、许可证兼容且覆盖目标平台的 crate；通用能力不重复造轮子，自研重点放在 RMCL 的领域规则和编排。
- 引入依赖时控制 feature、传递依赖和平台成本，不能只因流行就引入。
- 注释解释约束、原因和非显然行为，不复述代码，不保留注释掉的死代码。
- `NOTE:` 记录必须知道的约束或不变量。
- `SEE:` 指向相关规范、固定源码版本或详细文档。
- `FIXME:` 只标记已确认且尚未修复的缺陷，并写清触发条件或跟踪位置。
- `TODO:` 只用于明确、可执行的后续工作，不用于模糊的愿望清单。
- 公共 API 文档说明调用者需要知道的语义、错误和副作用；不为显而易见的 getter 或字段堆砌注释。
- 生产路径避免依赖 `unwrap()`、`expect()` 或 panic 处理可恢复错误；测试和真正不可恢复的不变量除外。
- 不记录账号令牌等秘密。诊断用的启动计划必须经过脱敏。

完整约定见 `docs/engineering/CONVENTIONS.md`。

## 独立实现与许可

- RMCL 原创代码按 MIT 路线准备。
- 第三方元数据、测试样本和依赖也要记录来源及各自许可。

见 `docs/engineering/LICENSING.md`。

## 分支与发布

- `main` 已启用分支保护，禁止直接 push；所有修改通过分支和 Pull Request。
- PR 只允许 Squash merge，合并后自动删除 head branch。
- PR 标题使用 Conventional Commits；Squash 后标题成为 `main` 的提交信息。
- PR 描述按 `.github/pull_request_template.md` 填写，写清目标、验证和 release notes 影响。
- 发布使用 release-plz release PR；不要手工直接改版本。流程见 [分支、PR 与发布](docs/engineering/RELEASING.md)。
- 合并前必须确认必需 CI 检查全部通过。

## Git 提交

- 每完成一个边界清晰且验证通过的任务，创建一次 git commit。
- 使用 Conventional Commits：`<type>(<scope>): <summary>`，其中 scope 可选。
- 常用类型包括 `feat`、`fix`、`docs`、`refactor`、`test`、`build`、`ci`、`perf`、`style`、`chore` 和 `revert`。
- 提交信息使用简洁、明确的英文祈使句摘要，例如 `docs: establish project decisions`。
- 提交前检查 `git status` 和暂存差异，只暂存本任务相关文件，不混入用户已有修改或无关生成物。
- 不为了凑提交而拆分相互依赖、无法独立验证的修改，也不把尚未完成的任务标记为完成。

## 验证要求

根据修改范围运行适用的检查：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
```

nextest 是项目的标准测试运行器；它不执行 doctests，因此最后一条命令不能省略。CI 的具体矩阵见 `docs/engineering/CI.md`。

- 纯解析、规则和计划生成使用普通单元测试或 golden tests。
- 网络测试默认使用本地 mock 和固定 fixtures，不依赖真实账号或外部服务。
- 真实游戏启动、图形界面和安装包需要在对应平台验证，不能用 CI 编译成功代替。
- 文档改动至少检查文件命名、本地链接和事实是否与当前代码一致。
