# 世界存档跨实例迁移与共享调研

日期：2026-10-30

状态：调研完成；已按“每实例独立存档 + 世界级复制/移动 + CoW”落地，最终规范见 [地图与世界存档](../../docs/architecture/WORLDS.md)

关联：[实例与版本隔离](../../docs/architecture/INSTANCES.md)、[地图与世界存档](../../docs/architecture/WORLDS.md)、[本地内容管理](../../docs/architecture/LOCAL_CONTENT.md)

## 结论摘要

1. **不要把同一个可变 `saves/` 目录软链接给多个实例作为默认方案。** 这不是“迁移”，而是让多个实例共同拥有同一个可变存档。新版打开旧世界会原地升级；旧版再打开时已经无法保证兼容。
2. **业界主流是“每个实例独立存档 + 显式的世界级复制/导入/导出”。** HMCL、Prism/MultiMC、PCL2、Modrinth App、ATLauncher 都是这个方向。链接只出现在高级复制选项里，而且通常明确排除 `saves/`。
3. **复制不等于必须付出完整磁盘成本。** APFS `clonefile`、Btrfs/XFS reflink、ReFS block clone 都能做写时复制；复制瞬间完成，之后两边独立演化。失败时回退普通复制即可。
4. **RMCL 应把“世界”做成一级对象**：扫描、预览、复制到其他实例、移动、导入、导出、备份、恢复、删除、兼容性检查。这样用户不需要打开 Finder/Explorer 搬文件夹。
5. **共享存档可以作为专家模式保留，但应链接整个 `saves/` 目录、记录链接关系、检查版本和模组差异、阻止删除目标，并默认关闭。** 不建议支持“只链接某一个世界”的细粒度软链接。
6. **长期可做“世界库/分支”**：世界保存在启动器管理的库中，实例按需 checkout/commit，每次升级前自动分支。它比实时共享更安全，但工程量大，不适合作为第一步。

## RMCL 当前状态

- 实例根目录是 `versions/<id>/`；隔离实例的 `saves/` 在实例根下，共享实例使用 `.minecraft/saves/`。见 `crates/rmcl-instance/src/instance.rs` 中的 `InstanceLayout::game_dir()` 和 `saves_dir()`。
- 从已有实例创建加载器实例时，`copy_instance_game_content()` 会复制源实例的顶层游戏内容，包括 `saves/`、`config/`、`options.txt`、`servers.dat` 等；这是整目录复制，没有世界级选择、冲突策略或兼容性检查。见 `crates/rmcl-app/src/instances.rs`。
- 旧实例迁移到隔离模式只复制 `mods/`、`resourcepacks/`、`shaderpacks/`，不复制 `saves/`。见 `migrate_instance_to_isolated()`。
- 目前没有世界扫描、世界导入/导出、世界备份、世界复制/移动或 `session.lock` 处理。
- `IsolationMode::Shared` 已经是一种粗粒度的共享存档模式：多个共享实例共用 `.minecraft/saves/`。它同样存在跨版本风险，但语义明确，用户知道自己选择了共享游戏目录。

## 业界做法

| 启动器 | 默认存档位置 | 跨实例转移 | 共享/链接 | 备注 |
| --- | --- | --- | --- | --- |
| HMCL | 实例 run directory 的 `saves/`，或共享根目录，或用户自定义 run directory | 复制实例时可选择是否复制存档；世界管理支持复制、导出、导入、备份 | 没有世界级链接功能；自定义 run directory 可让多个实例指向同一目录，但属于用户自担风险的粗粒度共享 | 世界列表默认按实例游戏版本过滤；`session.lock` 加锁；复制/备份排除 `session.lock` |
| Prism Launcher / MultiMC | 实例 `.minecraft/saves/` | 复制实例时可勾选 Copy saves；世界管理支持复制、导入、导出 | 高级复制支持符号链接、硬链接和 CoW clone；有 “Don't link saves” 选项；CoW 支持 APFS/Btrfs/Bcachefs/ReFS/XFS/ZFS | 1.20+ 需要处理 `allowed_symlinks.txt`；链接存档是高级选项，不是默认 |
| PCL2 | 版本隔离开启时在版本目录，关闭时在 `.minecraft` 根 | 手动复制或整合包导出 | 版本隔离是整目录级开关，不做 `saves/` 单独链接 | 删除隔离实例会同时删除其存档，界面会警告 |
| Modrinth App | 每个 profile 的 `saves/` | Worlds 标签页管理世界和服务器；备份 API | 没有跨 profile 共享存档功能 | 0.9.4 加入 Worlds 标签页 |
| ATLauncher | 每个实例的 `saves/` | 手动复制；实例导入/导出 | 没有世界级链接功能 | 提供一键存档备份 |
| 官方启动器 / CurseForge | 安装或 profile 的游戏目录 | 手动打开文件夹复制 | 官方启动器多个安装可指向同一游戏目录 | 没有世界管理 |
| Minecraft 本身 | `gameDir/saves/` | 无 | 1.20+ 校验世界目录内部符号链接；世界目录本身可被链接，但版本相关且容易踩坑 | 世界升级原地进行，降级会警告并建议备份 |

### HMCL 的具体行为

参考固定提交：`HMCL-dev/HMCL@0c3c7dd62ab073bf384fd0f57914388aae1359c4`。

- `HMCLGameInstance#getRunDirectory()` 调用 `HMCLGameRepository#computeRunDirectory()`：优先实例根，其次共享基础目录，最后用户自定义 run directory。
- `WorldListPage` 从 `instance.getSavesDirectory()` 扫描世界；默认只显示与实例游戏版本相同的世界，可通过 `showAll` 显示全部。
- `World` 解析 `level.dat` 的 `Data.LevelName`、`Data.Version.Name`、种子、图标等；`lock()` 对 `session.lock` 加文件锁；`copy()` 和备份任务都排除 `session.lock`。
- `HMCLGameRepository#duplicateInstance(src, dst, copySaves)` 的 `copySaves` 由 UI 询问，默认不复制存档。
- 世界管理提供复制、导出、删除和备份；备份是 `backups/<timestamp>_<world>.zip`。

这说明 HMCL 也没有用软链接解决“多个实例共用一张地图”。它提供的是世界级操作和可选的整目录运行目录。

### Prism Launcher 的具体行为

- 实例复制高级选项提供符号链接、硬链接、递归链接和 CoW clone。
- 界面明确提供 “Don't link saves”：勾选 Copy saves 时，存档会被复制而不是链接，避免两个实例共享同一个世界目录。
- 官方 Wiki 说明 CoW clone 只在 APFS、Btrfs、Bcachefs、ReFS、XFS、ZFS 等文件系统可用；FAT 不支持链接。
- 1.20 之后 Minecraft 会校验世界目录内部的符号链接，Prism 需要在 `allowed_symlinks.txt` 中登记目标；相对链接在部分版本不可用。

这实际上是在说：**链接可以作为省空间的复制手段，但不能作为共享可变存档的默认语义。**

## 为什么软链接不适合作为迁移方案

### 1. 世界升级是原地、单向的

Minecraft 在 `level.dat` 中记录 `Data.DataVersion`。新版本打开旧世界时会对区块和存档数据做 DataFixer 升级，并把新的 DataVersion 写回。升级通常不可逆；旧版本打开新世界会看到“World was saved in a newer version”警告，继续打开可能丢失方块、实体、进度，甚至损坏区块。

软链接让新实例直接修改旧实例唯一的那份世界。用户以为只是“在新实例里玩”，实际是把旧实例的存档升级了。旧实例之后可能再也打不开。

### 2. 模组、数据包和实验性功能会扩大不兼容面

即使 Minecraft 版本相同，不同加载器、不同 Mod 集合、不同数据包也可能改变方块、物品、实体、注册表和存档结构。一个世界在 A 实例中被 Mod 写入自定义数据，B 实例缺少对应 Mod 时可能丢数据或崩溃。

`level.dat` 的 `Data.WasModded`、`Data.DataPacks.Enabled/Disabled`、`Data.enabled_features` 可以提供线索，但无法完整证明兼容。

### 3. 并发写入没有可靠保护

Minecraft 用世界目录下的 `session.lock` 防止两个进程同时写同一个世界。1.16+ 使用 Java `FileChannel.tryLock()`。软链接如果指向同一个真实文件，锁仍然有效；但以下情况会绕过保护：

- 一个实例使用软链接，另一个实例使用复制出来的副本；
- 启动器在世界仍被游戏占用时复制/删除/覆盖；
- 网络文件系统或沙箱对文件锁支持不完整；
- 用户在游戏运行时手动改文件。

复制前必须尝试获取 `session.lock`，失败就拒绝操作。注意 Java NIO 在 Unix 上使用 `fcntl` 记录锁，不是 `flock`；Rust 侧如果只用 `flock`，不会和 Java 的锁互斥。Windows 上 Java 使用 `LockFileEx`，与 `fs2`/`fd-lock` 一类实现更容易兼容。实现时要按平台验证锁原语。

### 4. 1.20+ 对世界内部符号链接有额外校验

Minecraft 1.20 pre-release 7 起，世界目录内部的符号链接必须在 `allowed_symlinks.txt` 中登记，否则拒绝加载。世界目录本身仍然可以被链接，但行为随版本变化（例如 MC-264574 在 1.20.1 出错，1.20.2 修复）。相对链接不可靠（MC-263046）。Flatpak 等沙箱还需要额外授予目标路径访问权限。

因此：

- 链接整个 `saves/` 目录通常比链接单个世界更稳，但会共享所有世界；
- 链接单个世界需要 allowlist、绝对路径和版本适配，跨平台成本高；
- 这不是一个适合普通用户的迁移机制。

### 5. 硬链接比软链接更不可预测

硬链接不能链接目录，只能逐文件链接。Minecraft 保存不同文件的方式并不一致：`level.dat` 常用“写临时文件再 rename”的原子替换，region 文件可能原地写或重写，`session.lock` 原地写。结果是：

- 有些文件仍然共享 inode，写入会同时影响两边；
- 有些文件因为 rename 而自动分叉；
- 用户无法从界面上判断哪些数据共享、哪些已经分叉。

这对世界这种强一致的可变数据是灾难性的。硬链接适合不可变文件或备份去重，不适合世界目录。

### 6. 备份、导出、删除语义都会变得危险

- 导出工具可能不跟随符号链接，导出空目录；
- 删除实例时可能删掉链接目标，或者只删链接留下孤儿数据；
- 备份可能备份了链接而不是内容；
- 云同步工具可能把链接当成普通文件，或在两台设备间产生冲突副本；
- 用户看到两个实例里都有“同一个世界”，但不知道改动会互相影响。

## 方案对比

| 方案 | 版本安全 | 磁盘成本 | 实现复杂度 | 用户心智 | 建议 |
| --- | --- | --- | --- | --- | --- |
| 每个实例独立 `saves/`，手动复制 | 高 | 高 | 低 | 差，需要开文件夹 | 现状，应改进 |
| 世界管理器 + 普通复制 | 高 | 高 | 中 | 好 | 第一步 |
| 世界管理器 + CoW clone | 高 | 低 | 中 | 好 | 推荐 |
| 链接整个 `saves/` 目录 | 低 | 最低 | 低 | 中 | 仅专家模式 |
| 链接单个世界 | 低 | 最低 | 中 | 差 | 不做 |
| 硬链接世界文件 | 低 | 最低 | 中 | 差 | 不做 |
| 世界库 + 快照/分支 | 高 | 低 | 高 | 最好 | 长期 |

## 推荐设计

### 原则

- **不可变内容共享，可变内容复制。** `libraries/`、`assets/`、client JAR 按哈希共享；`saves/`、`options.txt`、`config/` 由实例拥有。
- **世界是一级对象，不是文件夹。** 用户操作的是“世界”，不是 `saves/xxx` 路径。
- **复制默认，移动显式。** 复制保留源世界；移动只在用户明确选择时执行，并在目标验证成功后删除源。
- **升级前备份，降级默认阻止。** 复制到更高版本时自动备份；打开比客户端新的世界时默认拒绝。
- **链接是专家选项，默认关闭。** 不用链接实现普通迁移。

### 世界模型

建议新增 `rmcl-world` crate（或先放在 `rmcl-instance`，等体量变大再拆）：

```rust
pub struct World {
    pub id: WorldId,              // saves/ 下的目录名，安全单路径组件
    pub path: PathBuf,
    pub name: String,             // Data.LevelName，显示时去掉 § 格式码
    pub game_version: Option<String>, // Data.Version.Name
    pub data_version: Option<i32>,    // Data.DataVersion
    pub last_played: Option<i64>,     // Data.LastPlayed
    pub was_modded: Option<bool>,     // Data.WasModded
    pub icon_png: Option<Vec<u8>>,    // icon.png，限制尺寸
    pub size_bytes: u64,
    pub locked: bool,                 // session.lock 是否被占用
}
```

`level.dat` 是 gzip 压缩的 NBT。只读取 `Data` 下的少量字段，不解码 region 文件。解析要有大小、深度和条目数上限，避免恶意世界包造成内存或 CPU 问题。

### 操作

- `scan(instance)`：扫描 `saves/*/level.dat`，跳过符号链接，按 `LastPlayed` 倒序。
- `import(instance, zip_or_dir, options)`：安全解压到 staging，识别 `level.dat`，处理同名冲突，最后原子发布。
- `export(world, zip)`：流式压缩整个世界目录，排除 `session.lock`，拒绝符号链接，支持进度和取消。
- `copy(world, target_instance, options)`：核心迁移操作。见下文。
- `move(world, target_instance, options)`：同一文件系统优先 `rename`；跨文件系统复制、校验、再删除源。
- `backup(world)` / `restore(backup, target)`：备份默认排除 `session.lock`；恢复前先备份当前世界。
- `delete(world)`：先尝试锁，失败拒绝；删除后刷新列表。
- `quick_play(world)`：1.20+ 通过 `--quickPlaySingleplayer <folder>` 直接进入世界；更早版本只启动游戏。

### 复制算法

1. 校验源世界目录是真实目录，不是符号链接；递归检查内部是否有符号链接。
2. 尝试获取源世界 `session.lock`；失败则提示“游戏可能正在使用该世界”。
3. 读取源世界 `DataVersion` 和目标实例 client JAR 内 `version.json` 的 `world_version`。
4. 根据兼容性决定警告/阻止/自动备份。
5. 在目标 `saves/` 同目录创建 staging（`.rmcl-world-<uuid>`）。
6. 逐文件复制：
   - 优先 CoW clone；
   - 不支持时普通复制；
   - 跳过 `session.lock`；
   - 不跟随符号链接；
   - 保留文件权限和修改时间（可选）。
7. `fsync` 文件和目录，`rename` staging 到最终目录。
8. 失败或取消时清理 staging，源和目标都不留下半个世界。
9. 复制完成后可以选择立即启动目标实例并 Quick Play 进入世界。

冲突策略：

- 默认 `Rename`：`World` → `World (1)`，同时更新 `level.dat` 的 `LevelName`？建议只改目录名，保留世界显示名，或询问用户。
- `Reject`：目标已存在同名世界时返回冲突。
- `Replace`：仅在用户明确确认后执行，先备份目标世界。

### 兼容性检查

目标客户端的 `world_version` 从 client JAR 内的 `version.json` 读取。RMCL 已经在资源包格式检查中读取 client JAR 的 `version.json`，可以复用同一路径。不要依赖实例目录外的版本 JSON，因为加载器实例可能通过 `jar` 或 `inheritsFrom` 重定向 client JAR。

判断：

| 源世界 DataVersion | 目标 client world_version | 行为 |
| --- | --- | --- |
| 相等 | 相等 | 直接复制/启动 |
| 更小 | 更大 | 允许；提示“将在目标实例中升级”；复制时自动备份 |
| 更大 | 更小 | 默认阻止启动；允许仅复制/导出用于归档，需明确确认 |
| 未知 | 任意 | 显示未知，启动前警告 |

模组维度：

- 比较源/目标实例 manifest 的 `loader` 和 `minecraft_version`。
- 比较启用 Mod 的文件名/元数据 ID 集合；不同则警告。
- 源世界 `WasModded = true` 且目标实例没有加载器时，强警告。
- 数据包/实验性功能不同时，至少显示警告，后续可读取 `DataPacks` 和 `enabled_features` 做更细判断。

### 界面

实例详情页新增“世界”标签，与 Mods / 资源包 / 光影并列：

- 世界卡片：图标、名称、最后游玩时间、游戏版本、DataVersion、大小、是否被占用。
- 操作：进入世界、复制到实例、移动到实例、导出 ZIP、备份、删除、打开文件夹。
- 兼容性徽章：可升级、同版本、需要更高版本、模组可能不兼容。
- 默认显示所有世界，但“需要更高版本”的世界禁用“进入世界”并给出原因；不要像 HMCL 那样默认隐藏，避免用户以为世界丢了。

“复制到实例”对话框：

- 目标实例选择器（排除源实例）。
- 世界多选。
- 兼容性预览和警告。
- 复制/移动、冲突策略、复制后是否启动。
- 进度、取消、结果汇总。

新建实例向导：

- 增加“从现有实例带入世界”步骤，按世界勾选，而不是无条件复制整个 `saves/`。
- 目标版本高于世界版本时，默认创建升级前备份。
- 从已有实例创建加载器实例时，默认仍可复制 Mod/配置；存档是否复制应单独询问。

删除实例：

- 如果实例包含世界，列出数量并明确“这些世界将随实例删除”。
- 提供“先导出全部世界”或“移动到其他实例”的入口。
- 如果 `saves/` 是专家模式的链接，只删除链接，绝不删除目标。

### 专家共享模式

如果确实要支持“多个实例共享同一份存档”，建议只允许链接整个 `saves/` 目录，并在实例 manifest 中记录：

```toml
[shared_saves]
target = "/path/to/shared/saves"
kind = "symlink" # 或 "custom_game_dir"
```

要求：

- 只允许用户显式开启，默认关闭，界面上标注“共享、非隔离”。
- 链接整个 `saves/`，不支持单世界链接；避免 1.20 世界内部符号链接校验和相对链接问题。
- 在游戏目录维护 `allowed_symlinks.txt` 时使用绝对路径，并保留用户已有内容。
- 启动前扫描共享世界，若存在 `DataVersion > client.world_version` 的世界，警告或阻止。
- 记录哪些实例链接到同一目标；删除任一实例时只移除链接，不删除目标；删除目标时列出受影响实例。
- 世界复制/删除/备份操作要识别链接，默认操作真实目标前要求确认。
- Windows 使用 junction 还是 symlink 要做能力检测；没有权限时明确报错，不静默回退到复制（否则用户以为共享了，实际没有）。
- Flatpak/沙箱/网络盘要给出额外限制说明。

即使这样，仍然建议把“共享存档”放在高级设置里，而不是主流程。主流程用世界管理器的复制/移动。

### 长期：世界库/分支

如果 RMCL 想在“同一张地图跨版本继续玩”上做得比 HMCL 更好，可以考虑世界库：

- 启动器数据目录维护一个世界目录，每个世界有稳定 ID、元数据和快照列表。
- “在实例中打开”实际是 checkout 一个快照到实例 `saves/`；可以使用 CoW clone 降低空间成本。
- 升级前自动创建快照/分支；旧实例继续使用旧分支，新实例使用新分支。
- 用户可以选择把某个实例的世界“提交”为新快照，形成版本历史。
- 不提供实时双向共享；世界不是可合并的数据，分支比合并更符合现实。

这比软链接复杂，但能真正解决“跨版本继续玩又不想破坏旧存档”的需求，而且天然支持回滚。可以作为后续差异化功能。

## 分期建议

### P0：世界模型与只读扫描

- 新增 `rmcl-world`（或先放 `rmcl-instance`）。
- 解析 `level.dat`、图标、大小、`session.lock` 状态。
- 实例详情页“世界”标签只读列表。
- 单元测试覆盖 NBT 解析、缺失字段、损坏文件、符号链接跳过。

### P1：复制/移动/导入/导出/备份

- 世界复制到其他实例，带兼容性检查和冲突策略。
- 复制使用 CoW clone + 普通复制回退；staging 原子发布。
- 导入 ZIP、导出 ZIP、备份/恢复。
- 长任务进入 `TaskManager`，支持进度和取消。
- 集成测试验证源世界不变、目标独立、失败清理。

### P2：新建实例向导与删除保护

- 新建实例时按世界选择复制。
- 删除实例前列出世界并提供导出/移动。
- Quick Play 进入复制后的世界。
- 共享实例的启动前版本警告。

### P3：专家共享模式（可选）

- 整 `saves/` 链接、manifest 记录、链接关系追踪、删除保护、`allowed_symlinks.txt`。
- 只在确认有真实需求后实现。

### P4：世界库/分支（长期）

- 快照、checkout/commit、分支、CoW 去重。

## 验证要点

- `level.dat`：现代/旧版字段、缺失 `Version`、损坏 gzip/NBT、超大文件、格式码名称。
- 兼容性：同版本、升级、降级、未知 DataVersion、`WasModded`、不同加载器。
- 复制：源不变、目标独立修改、`session.lock` 不复制、符号链接拒绝、同名冲突、取消/失败清理。
- 锁：游戏占用时拒绝；Unix 使用与 Java 兼容的 `fcntl` 锁；Windows 使用兼容的 `LockFileEx`。
- ZIP：路径穿越、符号链接条目、重复条目、超大条目、ZIP64。
- 平台：macOS APFS clone、Linux Btrfs/XFS reflink、Windows NTFS 普通复制回退。
- 真实游戏：在旧实例和新实例分别打开复制后的世界，确认旧实例世界未被升级；用 1.20+ Quick Play 验证直接进入。

## 待决策问题

- 新建实例时，存档默认复制还是默认不复制？建议默认不复制，但在向导中显式提供世界选择。
- 备份放在实例内 `backups/`，还是启动器数据目录？建议放启动器数据目录，避免删除实例时连备份一起删；CoW 快照则需与源同卷。
- 是否为“世界”引入稳定 ID，还是继续以目录名作为身份？目录名符合 Minecraft 习惯，但跨实例/重命名时容易冲突。
- 是否实现 `custom game directory`（HMCL 的 running directory）？它是比 `saves/` 链接更清晰的粗粒度共享方式，但会共享全部游戏内容。
- 是否值得实现专家共享模式，还是只做世界管理器 + CoW 复制？
- 世界库是否需要支持多设备同步？如果需要，冲突策略必须另行设计，不能简单依赖文件同步。

## 参考

- HMCL 固定提交 `0c3c7dd62ab073bf384fd0f57914388aae1359c4`：
  - `HMCLCore/src/main/java/org/jackhuang/hmcl/game/World.java`
  - `HMCL/src/main/java/org/jackhuang/hmcl/game/HMCLGameRepository.java`
  - `HMCL/src/main/java/org/jackhuang/hmcl/ui/instances/WorldListPage.java`
  - `HMCL/src/main/java/org/jackhuang/hmcl/ui/instances/WorldManageUIUtils.java`
  - `HMCL/src/main/java/org/jackhuang/hmcl/ui/instances/WorldBackupsPage.java`
- Prism Launcher Wiki, Instance Copy: <https://prismlauncher.org/wiki/help-pages/instance-copy/>
- Prism Launcher PR #833, Advanced Copy Options: <https://github.com/PrismLauncher/PrismLauncher/pull/833>
- Prism Launcher PR #1103, 1.20 allowed_symlinks.txt support: <https://github.com/PrismLauncher/PrismLauncher/pull/1103>
- MultiMC 0.4.8 world management: <https://multimc.org/posts/0-4-8-update.html>
- PCL2 功能介绍: <https://www.pcl2mc.com/features.html>
- PCL2 社区版版本隔离实现（行为参考）: <https://github.com/isHuaMouRen/PCL2-CE/commit/16e09c792ce8c13435fc6827e6da54170aaa3bc0>
- Modrinth App 0.9.4 Worlds 标签页: <https://modrinth.com/news/changelog/app/0.9.4>
- Modrinth App 世界 API: <https://github.com/modrinth/code/blob/f857d19a/packages/app-lib/src/api/worlds.rs>
- ATLauncher 一键存档备份: <https://flathub.org/en/apps/com.atlauncher.ATLauncher>
- Minecraft Wiki, Data version: <https://minecraft.wiki/w/Data_version>
- Minecraft Wiki, Java Edition level format: <https://minecraft.wiki/w/Java_Edition_level_format>
- Minecraft Wiki, Version.json: <https://minecraft.wiki/w/Version.json>
- Minecraft Wiki, Downgrading a world: <https://minecraft.wiki/w/Tutorial:Downgrading_a_world>
- Minecraft Help, Enable Symbolic Links: <https://help.minecraft.net/hc/en-us/articles/16165590199181>
- Minecraft 1.20 Pre-release 7 symlink validation: <https://feedback.minecraft.net/hc/en-us/articles/16224502352141>
- Mojira MC-264574, root world folder symlink: <https://bugs.mojang.com/browse/MC-264574>
- Mojira MC-263046, relative symlinks: <https://mojira.dev/MC-263046>
- Microsoft Learn, Block Cloning: <https://learn.microsoft.com/en-us/windows/win32/fileio/block-cloning>
- `reflink-copy` crate: <https://docs.rs/reflink-copy/latest/reflink_copy/>
- OpenJDK `FileDispatcherImpl.c`（Unix 使用 `fcntl`）: <https://github.com/openjdk/jdk19/blob/master/src/java.base/unix/native/libnio/ch/FileDispatcherImpl.c>
