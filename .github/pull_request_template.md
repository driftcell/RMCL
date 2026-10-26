### What does this PR do?

<!--
Describe the user-visible change and the task/issue it closes.
Link the task or issue if one exists.
-->

### How did you verify your code works?

<!--
List the exact commands and manual checks you ran.
Mention the platform and whether a real game/window was involved.
-->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo nextest run --workspace --locked`
- [ ] `cargo test --workspace --doc --locked`

### Release notes

<!--
User-visible changes, configuration migrations, breaking changes, or "none".
The PR title must remain a Conventional Commit because it becomes the squash commit.
-->
