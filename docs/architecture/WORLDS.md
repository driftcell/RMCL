# 地图与世界存档

状态：世界扫描、复制、移动、导入、导出、备份、恢复、删除、兼容性检查和 Quick Play 进入已实现；共享存档目录未实现

日期：2026-10-30

## 边界

`rmcl-world` 负责世界目录的发现、`level.dat` 元数据、世界复制/移动、ZIP 导入导出、备份与 `session.lock`。它不依赖启动器配置或 GPUI。

`rmcl-app` 负责跨实例编排、仓库写锁、升级前备份和长任务边界。`apps/rmcl` 只负责“地图”页面和交互。

不实现共享 `saves/` 软链接。每个实例拥有自己的世界目录；跨实例转移使用复制或移动。

## 布局

隔离实例：

```text
.minecraft/versions/<instance-id>/
├── saves/
│   └── <world>/
│       ├── level.dat
│       ├── icon.png
│       ├── region/
│       └── session.lock
└── backups/
    └── <timestamp>_<world>.zip
```

共享实例使用 `.minecraft/saves/` 和 `.minecraft/backups/`。`libraries/`、`assets/` 继续共享。

## 模型

`World` 从 `saves/<world>/level.dat` 读取：

- `Data.LevelName`：显示名称，去掉 `§` 格式码。
- `Data.DataVersion`：世界数据版本；缺失时回退到 `Data.Version.Id`。
- `Data.Version.Name`：最后保存该世界的游戏版本。
- `Data.LastPlayed`：最后游玩时间，Unix 毫秒。
- `Data.WasModded`：是否被模组客户端写过。
- `Data.DataPacks.Enabled`：启用的数据包。
- `Data.enabled_features`：实验性功能。
- `icon.png`：世界缩略图，限制读取大小。
- `session.lock`：是否被其他进程占用。

只解析 `level.dat`，不解析 `region/` 或区块 NBT。损坏的单个世界进入扫描警告，不隐藏整个列表。

世界 ID 就是 `saves/` 下的目录名，必须是安全的单路径组件。符号链接世界目录不会被跟随。

## 操作语义

### 复制

- 复制默认保留源世界；源世界在操作期间加 `session.lock`。
- 在目标 `saves/` 内创建 staging 目录，完整复制后原子 rename 发布。
- 跳过 `session.lock`，拒绝世界内部符号链接。
- 冲突策略：`Rename`（默认，生成 `World (1)`）、`Reject`、`Replace`。
- `Replace` 先备份目标世界，再把旧目录移开、发布新目录；失败时恢复旧目录。
- 复制优先逐文件 CoW clone，失败回退普通复制。
- 支持取消；失败或取消不会留下半个目标世界。

### 移动

- 同一文件系统优先 `rename`，原子完成。
- 跨文件系统复制、校验目标后再删除源。
- 移动前释放 `session.lock`，避免 Windows 无法重命名/删除打开中的目录。
- 源世界和目标世界相同时直接返回，不重复操作。

### 导入与导出

- 导入只接受包含一个合法世界的 ZIP；使用 `rmcl-archive` 安全解压，拒绝符号链接、路径穿越和重复条目。
- 导出为 ZIP，根目录使用世界文件夹名，排除 `session.lock`。
- ZIP 写入临时文件后原子发布；导出前检查 `session.lock`。

### 备份与恢复

- 备份为 `backups/<timestamp>_<world>.zip`，同名冲突时追加序号。
- 恢复复用导入流程，默认自动重命名冲突。
- 复制或移动到更高数据版本的实例时，自动在目标实例的 `backups/` 中保存升级前世界。

### 删除

- 删除前获取 `session.lock`；被游戏占用时拒绝。
- 只删除 `saves/<world>/`，不触碰实例其他内容。

## 兼容性

目标客户端的 `world_version` 从 client JAR 内的 `version.json` 读取。加载器实例通过解析后的 client JAR 路径读取，不使用实例 ID 猜测。

| 世界 DataVersion | 目标 world_version | 行为 |
| --- | --- | --- |
| 相等 | 相等 | 可直接进入 |
| 更小 | 更大 | 允许；标记“将升级”；复制/移动时自动备份 |
| 更大 | 更小 | 禁止 Quick Play 进入；仍可复制或导出归档 |
| 未知 | 任意 | 允许但显示“版本未知” |

世界卡片同时显示 `WasModded`、数据包和实验性功能标签。目标实例没有加载器而世界被模组写过时显示“需要模组”；跨实例转移时加载器不同显示“加载器不同”。这些是警告，不声称完全兼容。

## 锁

Minecraft 1.16+ 对 `session.lock` 使用 Java `FileChannel.tryLock()`：

- Unix 上是 POSIX `fcntl` 记录锁，不是 `flock`。`rmcl-world` 使用 `rustix::fs::fcntl_lock`。
- Windows 上是 `LockFileEx`，使用 `fs4` 的 `try_lock`。
- 操作成功后显式解锁，并由文件关闭兜底。
- Windows 删除/重命名目录前先释放锁。

## 界面

侧边栏新增“地图”，位于“实例”之后。页面只针对当前选中实例：

- 顶部摘要：实例、地图数量、总占用、目标 DataVersion。
- 工具栏：搜索、导入 ZIP、恢复备份、打开存档目录、刷新。
- 地图卡片：缩略图、名称、游戏版本、DataVersion、最后游玩、大小、占用/模组/数据包/实验性标签、兼容性标签。
- 操作：进入（Quick Play）、复制到其他实例、移动到其他实例、导出 ZIP、备份、删除。
- 复制/移动对话框按目标实例列出兼容性和加载器差异；同名世界提供“为副本”和“覆盖”两种选择。
- 操作期间显示文件/字节进度和取消入口。

Quick Play 使用 `--quickPlaySingleplayer <world>`，仅 1.20+ 游戏版本可用。低版本会提示无法快速进入。

## 当前限制

- 不支持共享 `saves/` 目录或跨实例软链接；这是明确的安全边界，不是遗漏。
- 不计算 Mod 集合指纹，只比较加载器类型/版本、世界 `WasModded` 和标签。
- 备份恢复通过选择 ZIP 完成；尚无备份列表、保留策略和自动清理。
- 不支持世界重命名、批量选择、拖放导入和导出整合包。
- 没有 MCP 世界工具；AI 助手暂不能操作地图。
- 真实 GPUI 窗口中的地图页面、Quick Play 和跨平台文件操作仍需在 macOS、Windows、Linux 逐平台验收。

## 验证

- `rmcl-world`：元数据解析、格式码清理、缺失目录、符号链接跳过、复制冲突、取消清理、替换、移动、删除、ZIP 导出排除 `session.lock`、导入回环、备份恢复、client JAR `world_version`。
- `rmcl-app`：实例世界扫描、目标数据版本、跨实例复制、升级前备份。
- `rmcl-launch`：现代版本使用 `--quickPlaySingleplayer`，旧版本拒绝。
- `rmcl`：类型检查和现有 UI 测试；真实窗口验收仍待执行。

调研与取舍见 [世界存档跨实例迁移与共享调研](../../notes/development/world_migration_and_sharing.md)。
