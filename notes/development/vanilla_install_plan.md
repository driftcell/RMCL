# Vanilla 安装计划实现

日期：2026-10-25

状态：已实现并通过本地与真实元数据验证

## 调研与范围

只读兼容梳理由 `opencode-go/deepseek-v4.1-flash` 完成，参考 HMCL 固定基线。实现与 fixtures 均为独立编写，没有复制 HMCL 源码或测试资源。

本次加入：

- `rmcl-instance`：标准目录、版本 ID、安全仓库路径和 natives 隔离。
- `rmcl-install`：Maven 坐标、纯 Vanilla 文件计划、native 提取计划和 asset objects 规划。
- `rmcl-minecraft` 补充：`Architecture`、`Sha1Hash`、`AssetCatalog`。

真实网络、下载执行、解压、Java 和启动均不在本次范围。

## 关键取舍

- 采用严格元数据错误，而不是复制 HMCL 中可能下载错误文件或让路径逃逸的容错。
- `jar != id` 在纯 Vanilla 规划阶段拒绝，未来继承场景另行定义。
- native 条目不下载基础 artifact；无当前 OS key 时跳过。
- 只有整个 `downloads` 缺失才允许 native Maven 回退，classifiers 明确缺项时报错。
- 普通 `Library` 与需要解压的 `NativeLibrary` 分开，现代 LWJGL 独立 native JAR 仍属于前者。
- 没有 SHA-1 的不同 URL 不能仅因目标相同就合并为候选，避免把不同内容当成可互换资源。
- asset objects 采用第二阶段规划并按 hash 去重，保留 size 与 SHA-1 供执行器校验。

最终行为见 `docs/architecture/INSTALL_PLAN.md` 与 `docs/compatibility/REPOSITORY_LAYOUT.md`。

## 尝试与修正

- 第一轮 61 个测试中，多个 `@` 的 Maven 坐标被错误接受。随后把 `@` 统一列为组件禁用字符，测试通过。
- 编译阶段发现坐标借用类型和普通 library 返回值类型不一致，直接修正类型后重新验证，没有绕过错误。
- Clippy 检出测试辅助函数的多余生命周期，按提示简化。
- 代码审查时补充了 base URL 按路径段百分号编码，防止 `#` 被解析为 fragment。
- natives 目录使用 OS/架构隔离，而不是固定 `natives`，防止跨平台复用时覆盖。

## 验证

本地完整检查：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：64 个测试全部通过。
- `cargo test --workspace --doc --locked`

另外使用仓库外的临时诊断程序读取真实 Mojang 元数据，只生成计划，不下载游戏文件：

- 当前 release `26.3`：68 个计划文件、0 个 native 提取计划、5147 个 asset objects。
- `1.7.10`：33 个计划文件、2 个 native 提取计划；686 个资产名称按 hash 去重为 674 个对象。

真实元数据不提交为 fixtures，真实网络 smoke test 不进入默认 CI。
