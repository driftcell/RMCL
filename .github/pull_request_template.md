### 这个 PR 做了什么？

<!--
说明用户可见的变化，并关联任务或 issue。
-->

### 如何验证代码有效？

<!--
列出实际运行的命令和手动检查，说明平台以及是否涉及真实游戏或窗口。
-->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo nextest run --workspace --locked`
- [ ] `cargo test --workspace --doc --locked`

### 发布说明

<!--
用户可见变化、配置迁移、破坏性变更；没有就写“无”。
PR 标题必须保持 Conventional Commits，因为 Squash 后会成为提交信息。
-->
