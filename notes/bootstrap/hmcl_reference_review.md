# HMCL 参考源码梳理

日期：2026-10-25

基线：`0c3c7dd62ab073bf384fd0f57914388aae1359c4`

范围：只读静态梳理，未运行 HMCL 或其测试

固定源码根链接：[HMCL@0c3c7dd](https://github.com/HMCL-dev/HMCL/tree/0c3c7dd62ab073bf384fd0f57914388aae1359c4)。以下路径均相对于该仓库根目录。

## 模块职责

| HMCL 模块 | 静态梳理结论 |
| --- | --- |
| `HMCLCore` | 游戏模型、仓库、下载任务、账号、Java 元数据、启动与进程监控。它并非完全 headless，部分核心类型依赖 JavaFX。 |
| `HMCL` | JavaFX 应用、设置持久化、镜像选择、Java 管理、启动编排、日志和崩溃界面。 |
| `HMCLBoot` | Java 版本与路径检查的引导层，再进入应用入口。 |

结论：RMCL 不按这三个名称机械拆包，而将 UI 调度、应用任务和纯领域逻辑重新分层。

## M1 源码索引

| 能力 | 主要参考路径 |
| --- | --- |
| 远程版本清单 | `HMCLCore/src/main/java/org/jackhuang/hmcl/download/game/GameRemoteVersion.java`、`GameRemoteVersions.java`、`GameInstanceJsonDownloadTask.java` |
| 本地 manifest 与继承 | `HMCLCore/src/main/java/org/jackhuang/hmcl/game/GameInstanceManifest.java`、`DefaultGameRepositorySnapshot.java`、`DefaultGameInstance.java` |
| 标准目录与事务边界 | 同包下 `DefaultGameRepositoryLayout.java`、`DefaultGameRepositoryDraft.java` |
| 平台规则与参数 | 同包下 `CompatibilityRule.java`、`OSRestriction.java`、`RuledArgument.java`、`Arguments.java`、`Library.java` |
| client、libraries、assets | `HMCLCore/src/main/java/org/jackhuang/hmcl/download/game/` 中 `GameDownloadTask.java`、`GameLibrariesTask.java`、`LibraryDownloadTask.java`、`GameAssetIndexDownloadTask.java`、`GameAssetDownloadTask.java` |
| 下载任务与缓存 | `HMCLCore/src/main/java/org/jackhuang/hmcl/task/` 中的 `FetchTask.java`、`FileDownloadTask.java`、`CacheFileTask.java`，以及 `HMCLCore/src/main/java/org/jackhuang/hmcl/download/DefaultCacheRepository.java` |
| HMCL 镜像策略 | `HMCL/src/main/java/org/jackhuang/hmcl/download/HMCLDownloadProvider.java`、`setting/DownloadProviders.java`、`setting/DownloadSource.java` |
| Java 检测与选择 | `HMCL/src/main/java/org/jackhuang/hmcl/java/JavaManager.java`、`JavaInfoUtils.java`，核心 `java/JavaInfo.java`、`game/JavaVersionConstraint.java` |
| Mojang Java 自动下载 | `HMCLCore/src/main/java/org/jackhuang/hmcl/download/java/mojang/`、`HMCL/src/main/java/org/jackhuang/hmcl/java/HMCLJavaRepository.java` |
| 离线账号 | `HMCLCore/src/main/java/org/jackhuang/hmcl/auth/offline/OfflineAccountFactory.java`、`OfflineAccount.java` |
| 启动与进程日志 | `HMCLCore/src/main/java/org/jackhuang/hmcl/launch/DefaultLauncher.java`、`launch/`、`util/platform/ManagedProcess.java` |
| 应用启动编排与脱敏 | `HMCL/src/main/java/org/jackhuang/hmcl/game/LauncherHelper.java`、`HMCLGameLauncher.java` |
| 依赖检查与修复 | `HMCLCore/src/main/java/org/jackhuang/hmcl/download/DependencyManager.java`、`DefaultDependencyManager.java` |

## 对 RMCL 的直接影响

1. 先实现 manifest、继承、规则和目录模型，再实现带副作用的下载执行。
2. 下载、安装与修复共享完整性判断和候选源机制，避免形成三套互不一致的逻辑。
3. Java 的“手动下载额外发行版”和“启动时自动获取 Mojang runtime”在 HMCL 中不是同一路径，RMCL 不应未经验证就混为一个 provider。
4. 离线账号的基本 profile/UUID 与皮肤、内置 Yggdrasil 服务可以分开推进，M1 不因此引入全部皮肤设施。
5. 启动前生成独立、可诊断且脱敏的计划；UI 不直接拼接命令。
6. 核心测试主动补 OS × arch × feature 规则、继承环、候选源失败、取消和启动参数矩阵。

## 修复语义注意事项

本次静态梳理显示，`DefaultDependencyManager.checkGameCompletionAsync` 组合 client、patch、assets 与 libraries 检查，但不同文件的校验强度并不相同。

`GameVerificationFixTask.java` 的名称容易造成误解：当前职责主要是处理 1.6 之前 Forge client JAR 的旧签名条目，并非通用游戏修复器。`LaunchManifestNormalizer.java` 的启动前内存修正也不等同于磁盘文件完整性检查。

因此，RMCL 的“修复”必须按文件类型定义检查与恢复行为，不能仅根据 HMCL 类名推断保证。

## 已有测试与缺口

可参考的测试集中在：

- `HMCLCore/src/test/java/org/jackhuang/hmcl/game/`
- `HMCLCore/src/test/java/org/jackhuang/hmcl/util/`
- `HMCLCore/src/test/resources/game-json/`
- `HMCL/src/test/java/org/jackhuang/hmcl/setting/`

本次未找到覆盖完整镜像顺序、网络续传、Java 选择矩阵、Mojang Java 自动下载、完整启动命令或离线账号启动流程的集中系统测试。这只是本次检索结果，不证明其他位置绝对不存在测试。

HMCL 测试样本可能受原项目许可约束。RMCL 可以参照行为独立构造 fixtures，但不直接批量复制。
