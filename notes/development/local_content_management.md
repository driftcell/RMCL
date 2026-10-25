# 本地内容管理取舍

日期：2026-10-25

状态：已形成实现决策，最终规范见 [本地内容管理](../../docs/architecture/LOCAL_CONTENT.md)

## 背景

实例隔离完成后，`mods/`、`resourcepacks/`、`shaderpacks/` 已经有明确落点。下一步需要让 UI 能扫描、导入、删除和切换这些内容，而不等待远程 API。

## 参考行为

HMCL：

- Mods 接受 `.jar`、`.zip`、`.litemod`，禁用以 `.disabled` 后缀表示；扫描根目录和一层子目录，但只对部分加载器生效。
- 资源包通过 `options.txt` 的 `resourcePacks` 控制；1.13+ 使用 `file/<name>`，旧版本使用纯名称。
- 光影包只管理文件，没有统一的启用开关。
- 导入同名文件通常直接覆盖，不提供统一的冲突策略。

## 决策

- 新建 `rmcl-content`，把内容模型、扫描和本地变更与 UI 分开；`rmcl-app` 只提供 `spawn_blocking` 异步包装。
- 默认导入冲突策略为 `Rename`，不覆盖用户已有文件；保留 `Reject` 供后续“另存为”对话框使用。
- Mod 禁用使用 `.disabled` 后缀，和社区/HMCL 兼容。
- 资源包启用状态以 `options.txt` 为唯一来源；只修改 `resourcePacks` 和 `incompatibleResourcePacks`，保留其他行，原子写回。
- 空资源包列表的格式按实例基础版本推断：1.13+ 和快照使用 `file/`，旧版本使用纯名称。该启发式在文档中明确为限制。
- 光影包没有启用状态，不伪造 Iris/OptiFine 配置。
- 扫描对符号链接、无效名称和不可读条目产生警告，不让单个坏文件隐藏整个列表。
- 删除和切换拒绝符号链接祖先，避免通过内容目录逃逸。

## 暂缓

- 远程搜索/下载、Mod 元数据解析、哈希识别、更新检查和依赖。
- 资源包 `pack_format` 兼容性判断。
- 文件夹导入、拖放、批量操作和整合包。
