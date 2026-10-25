# Minecraft 仓库布局

状态：当前核心行为已实现

日期：2026-10-25

## 标准目录

`rmcl-instance::MinecraftLayout` 以一个现有或未来的 `.minecraft` 为根，提供以下映射：

| 内容 | 路径 |
| --- | --- |
| 版本 JSON | `versions/<id>/<id>.json` |
| client JAR | `versions/<id>/<id>.jar` |
| libraries | `libraries/<repository-path>` |
| asset index | `assets/indexes/<assetIndex.id>.json` |
| asset objects | `assets/objects/<sha1 前两位>/<完整 sha1>` |
| natives | `versions/<id>/natives-<os>-<arch>` |

版本 JSON 的获取与原始字节保存不属于当前 `InstallPlan`；路径API 已经独立提供。

## Natives 目录

natives 持久目录名称不是 Mojang 统一规范。RMCL 选择按目标 OS 与架构隔离：

- `natives-windows-x86`
- `natives-linux-x86_64`
- `natives-macos-aarch64`

OS 名称使用 `windows`、`linux`、`macos`。架构使用 RMCL 归一化名称，例如 `x86`、`x86_64`、`arm`、`aarch64`，避免不同 JVM 或宿主架构共享并覆盖同一份 native 文件。

## 版本与 client JAR

版本 ID 必须是非空白、安全的单路径组件。当前 Vanilla planner 只接受 `jar` 缺失或 `jar == id`；`jar != id` 返回 `UnsupportedClientJarRedirect`，避免安装写入路径与后续启动读取路径不一致。

继承场景中的 client 重定向需要未来单独定义，不能通过静默覆盖另一个版本的 JAR 实现。

## 仓库相对路径

`RepositoryPath` 使用 `relative-path` 固定 `/` 分隔语义，并额外拒绝：

- 空路径、绝对路径和尾部 `/`
- 空组件、`.`、`..`
- 反斜杠、冒号和 Windows drive prefix
- 控制字符

版本 ID 和 asset index ID 还必须是单一路径组件且不能为空白。Unicode 与空格不会被归一化。

`RepositoryPath` 只负责安全相对路径。与 Maven base URL 组合时，`rmcl-install` 使用 `url` 的路径段 API 进行百分号编码，不能把 `#` 等字符误当成 fragment。
