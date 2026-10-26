# 服务器发现与热门服务器目录调研

日期：2026-10-26

状态：第一阶段已实现，等待真实窗口和跨平台验收

## 目标

在现有“服务器收藏 + 一键进服”之外，增加一个发现入口：展示公开的、玩家较多的 Java 版服务器，让玩家选择后自动创建匹配版本的实例并直接进入。

## 现有能力

- [服务器列表、状态与一键进服](../../docs/architecture/SERVERS.md) 已实现全局收藏、Java Edition Server List Ping、MOTD/人数/延迟、图标缓存，以及 Quick Play/legacy 进服。
- 收藏是启动器级配置，不绑定实例或账户；进服使用当前选中的实例和账户。
- 当前侧栏只有“服务器”收藏页，没有“探索/发现”页；Modrinth 内容页从实例详情进入。
- 实例创建支持 Vanilla、Fabric、Quilt、Forge、NeoForge；新实例默认隔离。
- 服务器地址由 `rmcl-domain::ServerAddress` 类型化处理；Quick Play 与 legacy 参数由 `rmcl-launch` 生成。

## 候选数据源

### minecraft-java-servers.com

- 文档：<https://minecraft-java-servers.com/api>
- 公开 REST API，JSON、CORS、匿名可用。
- 端点：
  - `GET /api/v1/servers`：分页列表，支持 `type`、`version`、`min_players`、`max_players`、`requires_mods`、`language`、`online`、`page`、`per_page`（最大 500）。
  - `GET /api/v1/server/{id}`：详情，包含 host、port、version、players、types、country、uptime、banner、website、description。
  - `GET /api/v1/server/{id}/status`：实时在线状态和玩家数。
  - `GET /api/v1/trending`：7 天增长最快的 25 个服务器，但列表项不含 host/port，需要再查详情。
- 关键字段：`host`、`port`、`version_min`、`version_max`、`requires_mods`、`mod_loader`、`mod_count`、`icon_url`、`short_description`、`players_online`、`players_shared`、`uptime`、`votes`、`url`、`vote_url`。
- 限制：匿名 25 请求/小时/IP，突发上限 40；免费 key 120/小时；付费 key 600/小时。响应带 `X-RateLimit-*` 和 `Retry-After`。
- 2026-10-26 实测：
  - `online=true&requires_mods=false&min_players=100&per_page=50` 返回 123 个匹配服务器。
  - `online=true&min_players=500` 返回 71 个。
  - 结果里会出现 Minehut 子服务器、Unicode 名称、`players_shared=true` 和 `requires_mods=null` 的条目。
  - Hypixel 的 `requires_mods` 是 `null`，`version_min/version_max` 都写 `1.21`，但实际支持 1.8 和 1.21；不能只依赖单个字段判断兼容性。
- 条款：<https://minecraft-java-servers.com/terms>。服务器由用户提交；条款禁止未经同意的商业使用，但没有明确的数据再分发许可。文档鼓励 launcher 集成，并建议渲染 `vote_url` 投票按钮。正式接入前应联系站点确认归因/使用方式，或只把它作为可替换的 provider。

### mcapi.tr

- 文档：<https://mcapi.tr/api-docs>
- 无 API key，JSON、CORS。
- `GET /api/v1/trends` 返回按趋势分排序的服务器，包含 `hostname`、`address`、`port`、`players_online`、`players_max`、`version`、`favicon`、`uptime_percentage`、`daily_growth`、`trend_score`。
- 默认 300 请求/分钟；status 120/分钟。
- 没有 `requires_mods`/`mod_loader` 字段，无法可靠判断原版客户端是否可加入。
- 条款写明内容归 MCAPI.TR 所有，未经书面许可不得复制、分发或修改。适合作为“趋势”展示，不适合作为直接安装的数据源。

### minecraftservers.cz

- 文档：<https://minecraftservers.cz/docs/api>，OpenAPI：<https://minecraftservers.cz/api/v1/openapi.json>
- 公开只读 JSON API，无鉴权；支持 `search`、`gamemode`、`sort=votes|players|newest`、`limit`（最大 100）。
- 字段：`name`、`ip`（可能含端口）、`description`、`version`、`gamemode`、`players`、`players_max`、`votes`、`url`。
- 面向捷克/斯洛伐克服务器，规模和地域都不适合作为全球“热门服务器”主数据源。

### 其他站点

- `minecraft-mp.com`：公开 API 只用于投票校验和服务器自己的投票数据，没有发现接口。
- `mcsrvstat.us`、`mcstatus.io`、`mcapi.us`、`mc-api.io`：只查询已知地址的状态，不提供热门服务器列表。
- `minecraftservers.org`、`minecraft.buzz`、Planet Minecraft：没有公开发现 API，不应抓取页面。
- `findmcserver.com` 是 Mojang/GamerSafer 的官方服务器列表，但站点有 Cloudflare challenge，没有公开 API；不要绕过保护抓取。
- `mc-api.io` 有 150 请求/分钟的无 key 状态接口，但同样只支持已知地址。

## 数据质量观察

- “热门”列表会混入推广位、Minehut 子服务器和共享玩家数的条目；需要过滤 `players_shared=true`、`*.minehut.gg`、`*.aternos.me` 等临时地址。
- `requires_mods=null` 既可能表示未知，也可能是原版可加入；不能把 null 当成需要 mod，也不能当成确定原版。
- 版本字段可能是 `Velocity 1.7.2-26.3`、`Requires MC 1.8 / 1.21` 等非标准字符串，必须优先使用结构化 `version_min`/`version_max`，并对无法解析的值降级为手动选择。
- 服务器图标在 API 中经常为空；可以用 RMCL 自己的 Server List Ping favicon 补齐。
- SLP 不能可靠暴露 `online-mode`、白名单或客户端 mod 要求；公共服务器大多要求正版账号，离线账户需要在 UI 明确提示。

## 已确定方向

2026-10-26 与维护者确认：

- 入口：现有“服务器”页增加“收藏 / 发现”两个 tab，不新增侧栏页面。
- 第一阶段：内置人工核验的精选服务器清单；状态、MOTD、人数、图标和延迟使用 RMCL 现有 SLP，不依赖第三方状态 API。
- Mod 服务器：发现列表展示并标注“需要客户端 Mod/整合包”，但不自动安装；只有原版可加入的条目提供“安装并进入”。
- 动态 API 留作第二阶段可替换 provider，不作为第一阶段的运行时依赖。

## 第一版精选清单（草案）

以下地址和版本来自 2026-10-26 的实时 SLP/列表数据。发布前需要重新核验地址、版本范围和是否需要客户端 Mod；推荐版本是优先安装的版本，不是硬性唯一版本。

| 名称 | 地址 | 推荐版本 | 标签 | 备注 |
| --- | --- | --- | --- | --- |
| Hypixel | `mc.hypixel.net` | `26.3` | Minigames, PvP | MOTD 标注 1.8/26.3；通常要求正版 |
| Complex Gaming | `bee.mc-complex.com` | `26.3` | SMP, Skyblock, Factions | 原版网络 |
| DonutSMP | `play.donutsmp.net` | `26.3` | SMP | API 名称为风格化 Unicode |
| PikaNetwork | `mc.pika-network.net` | `26.3` | Minigames, SMP | 支持 1.8–26.3 |
| CubeCraft | `play.cubecraft.net` | `1.21.11` | Minigames | 支持 1.20–1.21 |
| SunRealms | `mc.sunrealms.net` | `26.3` | SMP, Dungeons | 支持 1.7.2–26.3 |
| InsanityCraft | `join.insanitycraft.net` | `26.2` | Survival, Factions | 支持 1.8–26.2 |
| LemonCloud | `one.lemoncloud.net` | `26.3` | Skyblock, Factions | 支持 1.20–26.3 |
| Wynncraft | `play.wynncraft.com` | `26.3` | RPG | 使用服务器资源包，但不需要客户端 Mod |
| OPBlocks | `play.opblocks.com` | `26.3` | Skyblock, Prison | 支持 1.7.2–26.3 |
| Craft Down Under | `nohub.playcdu.co` | 按整合包 | Modded | 50+ 整合包；只展示并标记，不自动安装 |

- 2b2t 等无规则/内容敏感服务器暂不进入默认清单，可在后续分类或用户自定义列表中提供。
- 所有条目默认使用 Vanilla；推荐版本在安装对话框中可修改。
- 公共服务器大多使用 online-mode；离线账户只提示风险，不阻止尝试。
- 来源链接和第三方服务器免责声明固定展示；精选清单随应用版本发布，不依赖远程配置。

## 第一版纵向切片

1. 在核心层定义 `FeaturedServer` 模型：稳定 ID、名称、`ServerAddress`、推荐版本、可选版本说明、标签、简介、是否需要 Mod、来源链接。核心模型不依赖 GPUI。
2. 在“服务器”页增加“收藏 / 发现”切换；“发现”页显示精选服务器卡片、状态、人数、版本、标签和来源。
3. 状态刷新复用 `ServerStatusClient` 和图标缓存：打开“发现”页刷新一次，手动刷新全部；离开页面后停止自动刷新，避免后台轮询。
4. 原版条目提供“安装并进入”：打开确认对话框，预填推荐版本和 Vanilla；确认后创建隔离实例，安装完成后使用现有 `GameLaunchRequest` 携带 `ServerAddress` 启动并进入服务器。
5. Mod 条目只显示“需要 Mod/整合包”标签和来源链接，禁用自动安装；用户可先手动创建对应实例。
6. 没有实例、没有账户、已有任务或安装失败时给出明确原因；安装和启动进度继续只显示在底部任务栏。
7. 测试覆盖精选清单解析、版本选择、安装后启动参数、Mod 条目禁用、状态刷新取消和失败降级；真实 SLP 至少验证一个精选地址。

## 实现记录（2026-10-26）

- `crates/rmcl-app/src/featured.rs` 和 `crates/rmcl-app/assets/featured_servers.json`：编译时嵌入精选清单，校验 ID、地址、来源 URL、版本和 Mod 标记，并生成稳定的实例 ID。
- `apps/rmcl/src/app/mod.rs`：新增“收藏 / 发现”tab 状态、发现状态刷新、精选服务器收藏、安装确认对话框和 `pending_server_join`；`InstanceCreate` 完成后自动进入服务器。
- `apps/rmcl/src/app/ui/servers.rs`：新增发现 tab、精选服务器卡片、Mod 条目警告、官网链接和安装/收藏操作。
- `rmcl-app` 单元测试覆盖清单解析、地址/来源/版本校验、重复 ID 和实例 ID；应用层编译通过，现有 `apps/rmcl` 状态测试通过。
- 仍需真实窗口验证收藏/发现切换、状态刷新、安装对话框、安装完成后自动启动，以及 macOS、Windows、Linux 的逐平台进服。

## 暂不做

- 不自动抓取没有公开 API 的服务器列表站点。
- 第一阶段不接入第三方发现 API；不把网络错误变成空页面。
- 不自动安装客户端 Mod/整合包，不自动下载服务器资源包或执行服务器提供的文件。
- 不把“状态在线”当成“一定可以加入”；在线模式、白名单、版本和 mod 要求仍可能导致失败。
- 不在第一阶段把服务器地址持久化为实例默认进服目标；先完成一次性“安装并进入”。

## 剩余待决定

- “安装并进入”是否允许复用已安装的兼容实例，还是总是新建隔离实例？
- 离线账户是否允许继续尝试，还是只提示不阻止？（当前倾向只提示）
- 精选清单的更新方式：跟随应用版本发布，还是后续增加可选的远程清单？
- 第二阶段是否接入 `minecraft-java-servers.com`，以及是否需要先取得站点书面许可。
