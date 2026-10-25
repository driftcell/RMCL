# 实例与版本隔离

状态：实例 manifest、目录布局、隔离启动、旧实例迁移和实例创建已实现；加载器与内容管理见关联文档

日期：2026-10-25

## 边界

`rmcl-instance` 负责实例身份、实例目录、游戏目录语义和 RMCL 自有 manifest 的原子读写。它不下载游戏文件、不读取启动器配置，也不依赖 GPUI。版本 JSON、client JAR、libraries 和 assets 仍属于标准 Minecraft 仓库布局，见 [Minecraft 仓库布局](../compatibility/REPOSITORY_LAYOUT.md)。

实例 ID 就是 `versions/<id>/` 的目录名，因此一个实例同时是一个标准版本目录。显示名称和加载器信息保存在 manifest 中，不写入 Mojang 版本 JSON。

## 目录

```text
.minecraft/
├── versions/
│   └── <instance-id>/
│       ├── <instance-id>.json
│       ├── <instance-id>.jar
│       ├── rmcl.instance.toml
│       ├── natives-<os>-<arch>/
│       ├── mods/                  # 隔离实例
│       ├── resourcepacks/         # 隔离实例
│       ├── shaderpacks/           # 隔离实例
│       ├── saves/                 # 隔离实例
│       └── options.txt            # 隔离实例
├── libraries/                     # 所有实例共享
└── assets/                        # 所有实例共享
```

没有 manifest 的版本是 legacy 实例，继续使用共享游戏目录。manifest 存在但损坏时，扫描结果保留该版本并记录 `manifest_error`，同时按共享模式展示，避免静默改变游戏目录；启动前仍需修复或删除损坏的 manifest。

## Manifest

`schema_version = 1`，文件名为 `rmcl.instance.toml`：

```toml
schema_version = 1
name = "Fabric 生存"
minecraft_version = "1.21.1"
isolation = "isolated"
# [loader]
# kind = "fabric"
# version = "0.16.10"
# java = "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java"
# max_memory_mb = 4096
# window_width = 1280
# window_height = 720
```

字段：

- `name`：界面显示名称；空白时回退到实例 ID。实例详情页可修改，目录和版本 ID 保持不变。
- `minecraft_version`：实例基于的游戏版本；空白时回退到实例 ID。
- `isolation`：`isolated` 或 `shared`。新实例默认 `isolated`。
- `loader`：可选的 `fabric`、`quilt`、`forge` 或 `neoforge` 及版本。加载器由实例创建流程写入；Forge/NeoForge 在写入前先运行现代 installer processors。
- `java`：可选的 Java 可执行文件绝对路径。缺省时按游戏版本自动发现、选择或下载；显式路径在启动前重新探测，不满足版本、系统或架构要求时拒绝启动，不静默回退。
- `max_memory_mb`：可选的 `-Xmx` 值，单位 MiB，允许 256–262144；缺省使用 JVM 默认值。
- `window_width` / `window_height`：可选初始窗口尺寸，必须同时存在，允许 320–16384；缺省使用游戏默认。

manifest 通过同目录临时文件、`sync` 和原子替换写入。未知字段会被 serde 忽略，但重新保存时会丢失；schema 0 或高于当前版本会被拒绝。`java`、`max_memory_mb` 和窗口字段都是向后兼容的可选字段，不改变 schema 版本；旧版 RMCL 会忽略它们。

## 隔离语义

- `isolated`：游戏工作目录、`mods/`、`resourcepacks/`、`shaderpacks/`、`saves/` 和 `options.txt` 都位于实例目录。
- `shared`：游戏工作目录是 `.minecraft` 根，内容目录沿用全局位置。
- `libraries/`、`assets/`、版本 JSON 和 client JAR 始终位于共享仓库；隔离不复制这些文件。
- natives 目录始终按实例 ID 与平台隔离，位于 `versions/<id>/natives-<os>-<arch>/`。

启动计划中的 `${game_directory}` 来自实例游戏目录；`${library_directory}` 和 `${libraries_directory}` 固定来自共享 `libraries/`，不能继续从游戏目录推导。

## 安装与迁移

新安装只有在版本 JSON 原本不存在、且来源不是 `VersionSource::Existing` 时，才写入默认 `isolated` manifest。重新安装或修复一个已有 legacy 版本不会自动切换隔离，避免已有的全局 mods、资源包和光影突然不可见。

旧实例迁移由界面显式确认：

1. 校验版本目录存在且不是符号链接。
2. 将全局 `mods/`、`resourcepacks/`、`shaderpacks/` 中实例目录尚不存在的普通文件复制到实例目录；不覆盖实例文件，不跟随符号链接。
3. 创建标准内容目录。
4. 最后写入 `isolation = "isolated"` 的 manifest。

复制期间失败时 manifest 尚未写入，实例仍保持共享模式。全局内容不会被移动或删除。

## 启动与删除

- `InstallOutcome` 携带解析后的 `InstanceLayout`；`LaunchOptions::from_instance` 使用该布局构造游戏目录、natives、共享 libraries 和 client JAR。
- `GameLauncher::run` 和 `run_installed` 都通过 manifest 决定游戏目录；legacy 版本继续按共享目录启动。
- 删除实例只删除 `versions/<id>/`。隔离实例会同时删除其 mods、资源包、光影和存档；共享实例删除版本目录本身（通常为版本 JSON、client JAR 和 natives），共享 `libraries/` 与 `assets/` 不受影响。删除前要求版本 JSON 与 ID 匹配，并拒绝符号链接目录。

## 当前限制

- 版本继承合并、Fabric/Quilt/Forge/NeoForge 加载器安装和加载器启动合并已实现，见 [版本继承、加载器安装与实例创建](LOADERS.md)；legacy Forge installer 尚未实现。
- 尚未实现给已有实例追加、升级或移除加载器。
- 已支持实例级 Java、最大内存和窗口尺寸；任意 JVM/游戏启动参数和窗口位置尚未实现。
- 尚未实现实例复制、导入和导出；重命名只修改 manifest 显示名称，不改目录。
- 本地内容扫描与管理已由 [本地内容管理](LOCAL_CONTENT.md) 提供；远程下载和必需依赖解析见 [远程内容与 Modrinth](REMOTE_CONTENT.md)，可选依赖和整合包尚未实现。
- 旧实例迁移只复制三类内容目录，不复制 `options.txt`、`servers.dat` 或存档；需要保留全局游戏设置时由用户自行处理。
- 共享实例之间仍会共享内容目录；这是 `shared` 的明确语义，不是隔离失效。

## 验证

- `rmcl-instance` 覆盖隔离/共享路径、manifest 往返、缺失 manifest 回退、schema 拒绝和内容目录创建。
- `rmcl-app` 覆盖新安装写入隔离 manifest、重装 legacy 版本保持共享、修复不创建 manifest、manifest 控制启动目录、旧实例迁移复制不覆盖、缺失实例拒绝迁移。
- `rmcl-launch` 覆盖隔离实例的工作目录与 natives 目录、共享 `libraries/` 占位符和 client JAR 路径。
- `rmcl-app` 的真实假 JVM 集成测试验证隔离实例的快照启动在实例目录写入游戏文件。
- 本地内容扫描、导入、切换和删除由 `rmcl-content` 与 `rmcl-app` 测试覆盖，见 [本地内容管理](LOCAL_CONTENT.md)。
- 版本继承、Fabric/Quilt profile、实例创建和 loader library 下载由 `rmcl-install`、`rmcl-minecraft` 与 `rmcl-app` 测试覆盖，见 [加载器与实例创建](LOADERS.md)。
- 真实 GPUI 窗口中的隔离实例安装、加载器创建、迁移确认、内容导入和启动仍需在 macOS、Windows、Linux 逐平台验收。
