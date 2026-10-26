# 服务器列表、状态与一键进服

状态：服务器收藏、Java Edition Server List Ping、MOTD/人数/延迟、图标缓存、Quick Play/legacy 进服、精选服务器发现和“安装并进入”已实现；SRV、Bedrock、动态服务器目录和 `servers.dat` 导入导出尚未实现

日期：2026-10-26

## 边界

服务器收藏是 RMCL 的启动器级偏好，保存在全局 `config.toml`，不写入实例 manifest，也不直接读写游戏目录中的 `servers.dat`。游戏内服务器列表是 NBT 格式且隔离实例各自独立；RMCL 不覆盖用户游戏数据。

一键进服使用当前选中的实例和账户。服务器条目不绑定实例或账户；界面在启动前显示当前选择，切换到其他实例后同一收藏仍可用于进服。

## 配置

`rmcl-store` schema 3 新增服务器收藏（当前 schema 4 继续保留）：

```toml
[[servers]]
id = "0f8b0d0e-…"
name = "生存服"
address = "play.example.com:25565"
```

- `id` 是 RMCL 生成的稳定 UUID，用于状态和图标关联。
- `name` 是显示名称；留空时使用规范化后的地址。
- `address` 保留原始 `host[:port]` 字符串。单条地址无效不会阻止整个配置读取；界面在加载后解析并显示“地址无效”。
- 状态、MOTD、在线人数、延迟和图标路径都是瞬态数据，不写入配置。

读取 schema 1/2/3 时在内存中升级到 4，`servers` 默认空；更高 schema 继续拒绝，避免旧版本静默丢弃新字段。图标缓存位于配置文件同目录的 `cache/servers/`，文件名为 PNG 内容的 SHA-1。

## 地址解析

`rmcl-domain::ServerAddress` 支持：

- `play.example.com` → 默认 25565，保留“未显式指定端口”语义。
- `play.example.com:25566`
- `[::1]`、`[::1]:25566` 和裸 IPv6 `::1`
- 拒绝空地址、scheme、路径、查询、空白、`@`、零/越界端口和无效 IPv6。

未显式指定端口时，Quick Play 参数保留裸主机名，让游戏继续应用默认端口或未来的 SRV 语义；只有显式端口才拼成 `host:port`。

## 状态查询

`rmcl-app` 直接实现 Java Edition Server List Ping，不依赖通用 Minecraft 协议库：

1. TCP 连接默认端口 25565。
2. 发送 handshake（packet id `0x00`，protocol `-1`，主机、端口、next state `1`）。
3. 发送 status request。
4. 读取有长度上限的状态包，解析 `version`、`players`、`description`、`favicon`。
5. 延迟从发送请求到收到状态响应计算，界面标注为连接延迟。

- 默认整体超时 5 秒，响应上限 1 MiB。
- 页面刷新最多 4 个服务器并发；离开页面或开始新刷新会取消旧请求。
- 每次打开服务器页刷新一次；页面可见时每 60 秒自动刷新，手动刷新按钮立即触发。
- 单个服务器失败只在行内显示错误，不弹全局 toast；旧状态不伪装成在线。
- MOTD 是 chat component，支持 string、object、array、`extra` 和 `with`，只扁平化为纯文本并移除 `§` 格式码；不渲染颜色、click/hover 事件或翻译键。
- `players`、`version` 缺失时显示“人数未知/版本未知”，不伪造数据。

`favicon` 只接受 `data:image/png;base64,`，校验 64×64 PNG 头，限制 256 KiB，并按 SHA-1 原子写入缓存；UI 用普通图片元素显示，不执行远程 HTML 或脚本。

## 一键进服

`GameLaunchRequest` 和 `LaunchOptions` 携带类型化 `ServerAddress`，不使用任意字符串拼接 `additional_game_arguments`。

根据实例 manifest 中记录的基础 Minecraft 版本选择参数：

| 基础版本 | 参数 |
| --- | --- |
| `1.20` / `23w14a` 及更高，含对应预发布版和快照 | `--quickPlayMultiplayer <host[:port]>` |
| `1.19.4` 及更早 | `--server <host> --port <port>` |
| 旧 alpha/beta | legacy `--server`/`--port` |

- 加载器实例使用 manifest 的 `minecraft_version`，不能用 `fabric-1.21.1` 之类的实例 ID 判断。
- 无法识别版本时返回启动计划错误，不静默猜测。
- IPv6 Quick Play 使用 `[host]:port`；legacy `--server` 传裸主机，端口单独传。
- 服务器参数追加在游戏参数末尾，仍经过现有的取消、任务互斥、日志脱敏和退出状态处理。
- 在线模式服务器使用离线账户会被服务器拒绝；SLP 状态不能可靠暴露 `online-mode`，界面不把“在线”当作“一定可加入”。

## 界面

Sidebar“游戏”组新增“服务器”页，页面内分为“收藏 / 发现”两个 tab。

“收藏”：

- 显示当前选中的实例和账户；没有实例、账户或已有任务时禁用“进入”并给出提示。
- 支持添加、编辑、删除、搜索、刷新全部和状态刷新。
- 每行显示图标、名称、地址、MOTD、在线/上限人数、版本、连接延迟和在线/离线状态。
- 删除只移除 RMCL 收藏，不修改游戏内 `servers.dat`。
- 图标缺失或加载失败时回退到通用网络图标。

“发现”：

- 展示随应用发布的精选服务器清单，显示名称、标签、简介、地址、推荐版本、来源链接和实时状态。
- 原版条目提供“安装并进入”和标题行的星标收藏；已收藏时星标填充，再次点击取消收藏。需要 Mod/整合包的条目只显示警告和来源链接，不提供自动安装。
- “安装并进入”确认对话框展示服务器、推荐版本和第三方服务器提示；已存在对应实例时改为直接进入。
- 未选择账户时禁用安装并提示；安装和启动进度只显示在底部任务栏。
- 状态查询复用 SLP 和图标缓存，页面可见时只刷新当前 tab。

## 精选服务器发现

- 清单位于 `crates/rmcl-app/assets/featured_servers.json`，由 `rmcl-app::featured` 在编译时嵌入并解析；地址、来源 URL、ID、版本和 Mod 标记都在加载时校验。清单随应用版本发布，不做运行时远程配置。
- 每个条目包含稳定 ID、名称、地址、推荐 Minecraft 版本、版本说明、简介、标签、是否需要 Mod 和官网链接。
- 状态、MOTD、人数、图标和延迟复用现有 Server List Ping 与图标缓存。打开“发现”页或切换 tab 时刷新一次；页面可见时每 60 秒刷新当前 tab；离开页面或切换 tab 会取消旧请求。
- 原版条目确认后创建隔离 Vanilla 实例，实例 ID 为 `rmcl-<server-id>-<version>`，显示名称使用服务器名和版本。安装完成后自动使用当前账户和该服务器地址启动。
- 已存在同 ID 实例时直接复用，不重复创建。推荐版本固定在清单中；用户仍可在“新建实例”页创建其他版本的实例。
- 安装和启动仍由 `TaskManager` 持有。UI 只保存待进入的服务器，并在 `InstanceCreate` 完成后启动 `GameLaunch`；应用在安装完成前退出时不会自动启动游戏。
- Mod 条目只展示“需要 Mod/整合包”标签和官网链接，不自动安装 Mod/整合包。
- 离线账户允许尝试，但公共服务器通常使用 online-mode；界面提示风险，SLP 不能证明一定可加入。
- 精选清单的地址、版本和 Mod 要求需要在发布前人工复核。2026-10-26 的调研来源和候选数据见 [服务器发现调研](../../notes/development/server_discovery.md)；当前不调用第三方发现 API。

## 当前限制

- 只支持 Java Edition；Bedrock/RakNet（通常 UDP 19132）不在范围内。
- 尚未实现 `_minecraft._tcp` SRV 查询；需要显式 `host:port` 或默认 25565。
- 不导入/导出/同步游戏内 `servers.dat`。
- 不做后台常驻轮询；只有服务器页可见时自动刷新。
- 不解析富文本颜色、服务器图标以外的图片、SLP 返回的服务器标签或插件信息。
- 精选服务器清单随应用版本发布，不接入动态发现 API；地址、版本和 Mod 要求需要人工复核。
- 不自动安装需要客户端 Mod/整合包的服务器；这类条目只展示警告和官网链接。
- 不持久化每个实例的默认服务器；“安装并进入”只在创建/复用实例后启动一次。
- 安装确认对话框使用清单中的推荐版本；需要其他版本时使用“新建实例”页。
- 状态查询不认证，也不能证明白名单、Mod、版本、正版认证或代理允许加入。
- 不支持 authlib-injector 第三方认证服务器。

## 验证

- `rmcl-domain` 覆盖主机、端口、IPv6、默认端口和非法地址。
- `rmcl-app` 使用本地 TCP mock 覆盖握手/status、MOTD 扁平化、图标校验、超时和图标缓存；`RMCL_TEST_SERVER` 控制的真实服务器 smoke 标记 `#[ignore]`，不进入 CI。
- 2026-10-26 在 macOS ARM64 使用 `RMCL_TEST_SERVER=mc.hypixel.net` 运行真实 Server List Ping，连接与状态解析成功；该 smoke 只验证协议，不验证进服。
- `rmcl-launch` 覆盖 Quick Play/legacy 参数、1.20/23w14a 边界、预发布版、快照、显式端口和 IPv6。
- `rmcl-app` 假 JVM 集成测试验证服务器目标进入真实进程参数。
- `rmcl-store` 覆盖 schema 2/3 升级、服务器列表往返和无效地址仍可读取。
- `rmcl-app` 覆盖精选服务器清单解析、地址/来源 URL/版本/重复 ID 校验和实例 ID 生成。
- 真实窗口中的收藏/发现切换、精选状态刷新、“安装并进入”对话框和安装后自动启动仍需在 macOS、Windows、Linux 逐平台验收。
- 真实窗口中的添加/编辑/删除、图标显示、自动刷新和 1.20+/旧版实际进服仍需在 macOS、Windows、Linux 逐平台验收。
