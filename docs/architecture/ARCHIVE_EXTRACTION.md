# ZIP 安全解压

状态：当前核心行为已实现

日期：2026-10-25

## API 与执行方式

`rmcl-archive::ArchiveExtractor::extract_zip` 执行 `rmcl-domain::ExtractionPlan`，支持 ZIP/JAR 的 stored 与 deflate 内容。

该 API 是同步阻塞操作，调用方需要将其放入 `spawn_blocking` 或专用工作线程。`CancellationToken` 在条目之间和每个读取块时检查，进度回调在每个条目完成后触发。

`ExtractionPlan` 已从 `rmcl-install` 上移到 `rmcl-domain`，安装计划仍 re-export 同一类型，因此下载、安装与解压不形成反向依赖。

## 发布与目录策略

- 首版只发布一个新目录，目标已存在时返回 `DestinationExists`，不做合并或覆盖。
- 在目标父目录创建 `.rmcl-extract-*` staging，所有条目成功后才重命名到目标。
- 发布前再次检查目标是否出现，避免并发任务把两个结果混入同一目录。
- 失败、取消或 CRC 不符时删除 staging，不发布部分目录。
- 文件会 flush，但不承诺文件内容或目录项在系统崩溃时立即持久化。

已有目录的复用、清理、备份和合并属于实例或修复策略，不在通用 archive crate 中隐式执行。

## 条目与路径安全

仅允许普通文件和目录：

- 不创建符号链接、硬链接、FIFO 或设备节点。
- Unix 文件类型位异常的条目返回 `UnsupportedEntry`；同时检查原始 external attributes，标记为 `System::Dos` 但携带 Unix 特殊类型的条目也会被拒绝。
- 绝对路径、`..`、`.`、空组件、反斜杠、盘符、冒号和控制字符均拒绝。
- 重复目标直接报错。
- 为统一 Windows、macOS 和 Linux 行为，冲突键使用小写形式；仅大小写不同的条目也视为冲突。
- 实际写出的名称保留原始大小写，不进行 Unicode 归一化。

`exclude_prefixes` 使用元数据原始名称的字符串前缀匹配，在路径校验后、类型检查与写入前应用。不会额外执行 GLFW、SDL、OpenAL 或扩展名相关的启动器私有过滤。

## 完整性与限额

默认 `ExtractionLimits`：

- 最多 10000 个条目
- 单条目最大 256 MiB
- 总展开大小最大 1 GiB

每个普通文件都同时验证：

- 实际展开字节数等于条目声明 size
- 实际内容的 CRC-32 等于条目声明值
- 单条目和总展开大小没有超过限制

限制在声明阶段和实际读取阶段都执行，避免只依赖不可信的中央目录元数据。

## 权限与结果

Unix 上保留普通文件的 rwx 权限位，不保留 setuid、setgid 或其他特殊位；其他平台不应用 Unix 权限。

`ExtractionReport` 返回目标、普通文件数、目录数、展开字节数和排除条目数。`ExtractionProgress` 返回当前条目、已完成/总条目数和已展开字节数。

## 当前不包含

- TAR、tar.gz 或其他归档格式
- 加密 ZIP、链接解压和嵌套归档
- 已有目录合并、覆盖或回滚策略
- natives 的复用、清理和启动参数生成
