# HMCL 参考基线

状态：已固定

日期：2026-10-25

## 上游与提交

- 上游仓库：`https://github.com/HMCL-dev/HMCL`
- 本地只读参考：`../HMCL`
- 基线 commit：`0c3c7dd62ab073bf384fd0f57914388aae1359c4`
- 提交时间：`2026-10-25T03:04:20+08:00`
- 提交摘要：`chore: Bump commonmark to 0.31.0 & Update deps.json (#7056)`
- 固定链接：[HMCL@0c3c7dd](https://github.com/HMCL-dev/HMCL/tree/0c3c7dd62ab073bf384fd0f57914388aae1359c4)

## 使用方式

- 该提交是功能行为、兼容性和测试对照的初始基准，不自动跟踪 HMCL `main`。
- 修改基线必须是有意识的操作：记录需要的新行为或修复、更新本文，并检查相关 fixtures 与兼容性结论。
- HMCL 仅作为参考，不直接复制或翻译其实现与资源，具体约束见 [独立实现与许可](../engineering/LICENSING.md)。
- RMCL 优先采用自己的模块边界。HMCL 的 `HMCLCore`、`HMCL`、`HMCLBoot` 名称和依赖关系不直接映射为 Rust crate。
- 静态源码阅读只能证明源码中存在相应路径和逻辑，不能代替真实行为验证。

## 当前参考范围

初始梳理覆盖 M1 所需的 manifest、继承与规则、游戏资源下载、任务与镜像、Java、离线账号、启动与日志、依赖修复。详细源码索引及调研限制见 [HMCL 参考源码梳理](../../notes/bootstrap/hmcl_reference_review.md)。

加载器、整合包、内容平台和完整界面功能尚未建立完整台账。固定基线不代表 RMCL 已经实现或验证这些功能。
