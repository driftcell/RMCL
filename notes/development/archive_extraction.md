# ZIP 安全解压实现

日期：2026-10-25

状态：已实现并通过本地与真实 native 验证

## 调研与范围

只读解压行为梳理由 `opencode-go/deepseek-v4.1-flash` 完成，参考 HMCL 固定基线。HMCL 的静默重复覆盖、同大小跳过、直接写最终目录、私有 GLFW/SDL/OpenAL 过滤和清理策略没有进入通用实现。

本次将 `ExtractionPlan` 上移到 `rmcl-domain`，新增 `rmcl-archive`，首版只支持 ZIP/JAR 和 staging 后发布新目录。

## 实现取舍

- 使用当前 `zip 9.1.0` 的最小 deflate feature，并直接用 `crc32fast` 独立计算每个文件的 CRC。
- 使用 `relative-path` 固定条目路径语义，再增加 RMCL 的严格组件、盘符和控制字符校验。
- 重复与大小写冲突均报错，避免平台决定最终文件集。
- `ZipFile::is_file` 不作为唯一类型保证，额外检查 Unix 类型位，拒绝 FIFO 等特殊条目。
- 不合并已有目录，避免把覆盖、备份和修复策略混入底层解压。
- 同步 API 明确由调用方放到工作线程；取消在条目间和读取块间生效。

## 尝试与修正

- 进度测试最初漏算显式目录条目，修正测试期望为真实归档条目数。
- 测试 symlink 最初没有真正生成 symlink。查阅 zip 9 的外部属性 API 后，改用 `external_attributes` 构造真实 symlink，验证拒绝路径。
- 审查特殊文件时发现 `is_file` 不等于普通文件保证，补充 Unix 类型位检查和 FIFO 回归测试。
- CRC 测试通过修改 stored 条目内容但保留原元数据，确认损坏内容不会发布。

## Windows CI 回归与修复

首次跨平台 CI 中，Linux 和 macOS 通过，Windows 的 symlink 与 FIFO 两个测试失败。原因是 zip 在 `System::Dos` 分支会把外部属性解释为普通文件或目录，即使高 16 位携带 Unix 特殊类型；仅依赖 `is_symlink()` 或 `unix_mode()` 会漏检。

修复为同时检查原始 external attributes，并让特殊类型测试显式构造 `System::Dos` 条目。修复后本地 100 个测试再次全部通过。后续 [CI 运行 40163753569](https://github.com/driftcell/RMCL/actions/runs/40163753569) 中，Ubuntu、macOS 和 Windows 也已全部通过。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo nextest run --workspace --locked --profile ci --test-threads num-cpus --no-tests=fail`：100 个测试全部通过。
- `cargo test --workspace --doc --locked`

真实 native smoke test 使用 `1.7.10` 的计划和仓库外临时程序：

- `rmcl-net` 下载 native JAR，长度 571424 字节。
- `rmcl-archive` 成功解出 4 个文件，总展开大小 1600657 字节。
- 使用临时目标目录，没有修改项目或用户 `.minecraft`。

真实服务验证不进入默认 CI。

## 当前限制

尚未实现 TAR、已有目录合并、链接解压和启动阶段的 natives 复用策略。
