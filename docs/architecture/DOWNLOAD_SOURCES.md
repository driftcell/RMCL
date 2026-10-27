# 下载源与国内镜像

状态：Auto/Official/BMCLAPI 配置、启动探测、逐请求回退已实现

日期：2026-10-27

## 目标

RMCL 的主要用户在中国大陆，Mojang 官方下载源和 Java runtime 在大陆网络中经常超时。客户端优先使用国内镜像，同时保证镜像不可用时快速回到官方源，且任何镜像都不能绕过大小和哈希校验。

## 配置

`config.toml` 的 `download_source` 有三个值：

| 值 | 行为 |
|---|---|
| `auto`（默认） | 启动时探测 BMCLAPI；可用且足够快时优先镜像，否则只用官方源 |
| `official` | 不使用镜像 |
| `mirror` | 强制优先 BMCLAPI；每次请求仍保留官方源作为回退 |

设置页的“下载源”下拉框修改后立即写入配置并切换运行时状态，不需要重启。界面显示探测结果（可用延迟或回退原因），并按 BMCLAPI 使用要求标注“国内镜像由 BMCLAPI 提供”。

## 镜像与 URL 映射

当前内置镜像为 [BMCLAPI](https://bmclapidoc.bangbang93.com/)，基址 `https://bmclapi2.bangbang93.com/`。只重写已知官方主机，绝不把版本或加载器元数据中的任意 URL 改写到镜像。

| 官方主机 | 镜像路径 |
|---|---|
| `launchermeta.mojang.com`、`launcher.mojang.com`、`piston-meta.mojang.com`、`piston-data.mojang.com` | 保留路径，只替换主机 |
| `resources.download.minecraft.net` | `/assets` + 原路径 |
| `libraries.minecraft.net` | `/maven` + 原路径 |
| `maven.fabricmc.net`、`maven.minecraftforge.net` | `/maven` + 原路径 |
| `files.minecraftforge.net` | `/maven` + 原路径；已经是 `/maven/...` 时只替换主机 |
| `maven.neoforged.net/releases/...` | `/maven` + 去掉 `/releases` 后的路径 |
| `meta.fabricmc.net` | `/fabric-meta` + 原路径 |

未映射的主机保持官方源，包括：

- `meta.quiltmc.org`、`maven.quiltmc.org`：探测时 BMCLAPI 未提供可用内容；
- `maven.neoforged.net/api/...`：BMCLAPI 未镜像该 API；
- Modrinth API/CDN、Mojang 账号、皮肤纹理、GitHub Release。

## 探测

`rmcl-net::MirrorHealth` 在启动时和切换下载源时调用 `probe_default`：

1. 请求并读取镜像的 `mc/game/version_manifest_v2.json`；
2. 总超时 2 秒，客户端请求超时 3 秒；
3. 响应成功且耗时不超过 1500 ms 时，`auto` 才启用镜像；
4. 失败或超时则 `auto` 使用官方源；`mirror` 仍保持镜像优先并依赖逐请求回退。

探测结果通过 `AtomicBool`/`AtomicU64` 共享。安装器、版本目录、Java runtime、加载器元数据和 Forge/NeoForge 安装器都持有同一个 `Arc<MirrorHealth>`，因此切换设置后后续请求立即使用新状态。设置页的刷新按钮可随时重新探测。

## 回退与完整性

- 每个 `DownloadRequest` 的候选顺序是“镜像、官方”；`DownloadExecutor` 对镜像只尝试一次，随后立即回退官方源并使用正常的重试预算，避免镜像故障拖慢安装。
- Java runtime 的索引、manifest 和 raw 文件同样先生成镜像候选，再尝试官方 URL；官方 URL 仍先通过 `UrlPolicy` 校验。
- 加载器元数据和 Forge `.sha1` sidecar 在镜像失败后回退官方源。
- 所有游戏文件继续校验 SHA-1 和大小；镜像只改变候选来源，不改变完整性规则。
- 镜像 URL 只由 `MirrorSource` 的固定映射生成，不接受元数据中的任意镜像地址。

## 2026-10-27 存活探测

以下 URL 在同一日从开发机实际请求，官方与镜像均返回 HTTP 200：

| 资源 | 官方 | 镜像 |
|---|---|---|
| 版本清单 | `launchermeta.mojang.com/mc/game/version_manifest_v2.json` | 通过 |
| 版本 JSON | `piston-meta.mojang.com/v1/packages/.../b1.8.1.json` | 通过 |
| 旧版 client JAR | `launcher.mojang.com/v1/objects/.../client.jar` | 通过 |
| 新版 client JAR | `piston-data.mojang.com/v1/objects/.../client.jar` | 通过 |
| 资源对象 | `resources.download.minecraft.net/b6/...` | 通过 |
| 普通 library | `libraries.minecraft.net/org/lwjgl/...` | 通过 |
| Fabric 元数据 | `meta.fabricmc.net/v2/versions/loader/1.21.1` | 通过 |
| Fabric Maven | `maven.fabricmc.net/net/fabricmc/...` | 通过 |
| Forge 元数据 | `files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json` | 通过 |
| Forge Maven | `maven.minecraftforge.net/net/minecraftforge/...installer.jar` | 通过 |
| NeoForge Maven | `maven.neoforged.net/releases/net/neoforged/neoforge/...` | 通过（镜像重定向到 USTC） |
| Java runtime 索引 | `piston-meta.mojang.com/v1/products/java-runtime/.../all.json` | 通过 |
| Java runtime 文件 | `piston-data.mojang.com/v1/objects/.../COPYRIGHT` | 通过 |

Quilt 元数据/Maven 和 NeoForge API 在探测时镜像返回 404，因此没有加入映射。

## 限制

- 探测只覆盖镜像元数据端点，不逐个测速资源对象或 Maven 后端；镜像在中途故障时依赖单请求回退。
- `DownloadExecutor` 的请求超时仍按大文件设置；如果镜像接受连接后长时间不返回，回退可能比连接失败慢。
- 暂不支持用户自定义镜像地址或镜像顺序；后续可在 `MirrorSource` 之上增加候选列表和持久化配置。
