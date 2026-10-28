# 正版账号、令牌与皮肤

状态：Microsoft 设备代码登录、令牌刷新、账户删除、官方皮肤头像、皮肤页、PNG/图片直链换肤、恢复默认皮肤、本地皮肤库和 MCP 本地库接口已实现

日期：2026-10-28

## 边界

正版登录按职责拆分：

- `rmcl-auth`：无 UI 的 Microsoft/Xbox/Minecraft 协议、设备代码流程、令牌刷新、账户身份与错误类型。
- `rmcl-platform`：操作系统凭据存储的通用 `SecretStore` 接口和 Keychain / Credential Manager / Secret Service 适配。
- `rmcl-store`：`config.toml` 中的账户元数据，不保存任何令牌。
- `rmcl-app`：登录与启动所需的用例编排、凭据读写、皮肤下载与头像缓存、皮肤来源校验、本地皮肤库和 CPU 角色预览渲染。
- `rmcl-launch`：消费已解析的 `LaunchAccount`，只负责生成启动参数和进程。
- `apps/rmcl`：GPUI 对话框、账户列表、删除确认、头像展示、皮肤页、共享皮肤资料缓存和预览视图。

核心 crate 不依赖 GPUI；网络和钥匙串调用都通过 Tokio runtime 执行，GPUI task 只 await `JoinHandle` 并更新界面。

## Microsoft 登录

### 设备代码流程

使用 OAuth 2.0 device code grant：

- 授权端点：`https://login.live.com/oauth20_connect.srf`
- 令牌端点：`https://login.live.com/oauth20_token.srf`
- scope：`XboxLive.signin offline_access`
- 默认公开客户端 ID：`00000000402b5328`

应用请求设备代码后，界面显示一次性代码并打开 `https://www.microsoft.com/link`。后台按服务端给出的 interval 轮询令牌端点：

- `authorization_pending` 继续等待。
- `slow_down` 将间隔增加 5 秒。
- 设备代码过期、用户拒绝、取消或窗口关闭都会终止等待。
- 临时网络错误最多连续重试 3 次；其他错误直接返回类型化错误。

登录完成后的交换链：

1. Microsoft access token → Xbox Live user token（RPS，`d=<token>`）。
2. Xbox Live token → XSTS token（RelyingParty `rp://api.minecraftservices.com/`）。
3. XSTS token + user hash → Minecraft access token。
4. 查询 `entitlements/mcstore`，必须包含 `game_minecraft` 或 `product_minecraft`。
5. 查询 `minecraft/profile`，取得 profile UUID、玩家名和当前激活皮肤 URL。

没有 Xbox 档案、儿童账户、地区限制、封禁和未购买 Java 版都有独立错误语义；Xbox `XErr` 保留原始代码和重定向地址供界面解释。

### 令牌生命周期

`MicrosoftCredentials` 只保存：

- Microsoft refresh token；
- 最近的 Minecraft access token；
- Minecraft token 的绝对过期时间。

Microsoft access token 不持久化。Minecraft token 剩余有效期不足 5 分钟时视为过期，后续启动会重新执行 refresh token → Xbox → XSTS → Minecraft 交换。服务端轮换 refresh token 时，新值覆盖旧值；服务端未返回新 refresh token 时保留原值。

refresh token 失效或 Minecraft services 拒绝 token 时返回“需要重新登录”，不会静默用离线身份启动。

### 凭据存储

- 服务名：`com.rmcl.launcher`
- 键：`microsoft:<无连字符 profile UUID>`
- 值：版本化的 JSON `MicrosoftCredentials`
- macOS 使用 Keychain，Windows 使用 Credential Manager，其他 Unix 使用 Secret Service。

令牌不写入 `config.toml`、日志或启动计划诊断输出。`Debug` 实现会对 refresh token、Microsoft access token、Minecraft access token 和 Xbox/XSTS token 做脱敏。

删除正版账户时先删除系统凭据，再移除配置元数据并清除内存头像；凭据删除失败时保留账户并提示错误，避免出现“账户已消失但令牌仍留在钥匙串”的状态。

### 启动集成

`apps/rmcl` 在启动前解析正版账户：

- Minecraft token 仍有效：直接构造 `LaunchAccount`，不访问网络。
- token 过期：刷新并重新获取 profile，保存新凭据，再构造 `LaunchAccount`。
- 刷新后的玩家名变化会同步账户元数据和选中项。
- 若刷新成功但凭据无法写回钥匙串，本次仍可启动，并提示下次可能需要重新登录。

`LaunchAccount` 对正版账户使用 `user_type = "msa"`、真实 Minecraft access token 和 `user_properties = "{}"`。离线账户继续使用每次生成的随机 UUID token。

## 官方皮肤头像

Minecraft profile 的激活皮肤 URL 保存到 `microsoft_accounts[].skin_url`，仅用于元数据。下载与处理规则：

- 只允许 `textures.minecraft.net` 主机和 HTTP/HTTPS；重定向后的最终主机也必须匹配。
- 单次响应上限 1 MiB。
- 只接受 PNG；宽至少 64、高至少 32，且宽度必须是 64 的倍数以支持高分辨率皮肤。
- 从基础面部 `(8,8)` 取 8×8 像素，按 alpha 合成帽子层 `(40,8)`，再以最近邻放大到 64×64。
- 缓存文件位于配置目录的 `cache/skins/<sha1(url)>.png`，原子写入。
- 头像只用于本地展示；不访问第三方皮肤站，也不上传递增信息。

侧栏账户卡片、账户列表和主页账户下拉使用缓存头像；没有正版皮肤、离线账户或图片加载失败时显示玩家名首字母。

旧版本创建的正版账户可能没有 `skin_url`。应用启动时会在后台使用缓存的 Minecraft token 刷新 profile；该请求失败不会影响启动，只是继续显示首字母。用户重新登录或令牌自然刷新后会再次同步。

## 皮肤页与更换

侧栏“皮肤”页只对当前选中的正版账户生效；离线账户显示要求登录正版账户的空状态。页面资料按账户缓存在应用层 `SkinService` 中，离开页面再回来不会重复请求；手动刷新会保留旧资料直到新请求完成。

profile 解析现在保留：

- 全部 owned skins，包括 `ACTIVE` / `INACTIVE`、URL、模型和 alias；
- 全部 owned capes 及其激活状态；
- 当前激活皮肤的模型用于预览和模型提示。

换肤使用：

- `POST /minecraft/profile/skins`：multipart PNG + `variant=classic|slim`，返回更新后的 profile；
- `POST /minecraft/profile/skins`：JSON `{variant, url}`，协议层已支持，供后续验证官方 owned skins 的 URL 激活；当前页面统一走“下载校验后 multipart 上传”；
- `DELETE /minecraft/profile/skins/active`：恢复默认皮肤，返回更新后的 profile 或空响应。

PNG 与链接共用同一条候选流程：

- 官方 Java 上传只接受 64×64 或旧版 64×32 PNG；HD 皮肤只用于展示，不直接上传。
- 本地文件先限制 1 MiB，再解码校验；超过官方 24 KiB 限制时重新编码为规范 PNG，仍超限则拒绝。
- 图片直链只接受 HTTPS；拒绝 URL 凭据、私网/回环/链路本地地址，限制重定向次数、响应大小和超时。
- 不解析皮肤站网页，只接受返回 `image/png` 的图片直链；防盗链或 HTML 页面会提示改为保存 PNG 后导入。
- 链接由 RMCL 下载、校验并预览，再由 multipart 上传字节，保证“预览的就是上传的”；不依赖 Mojang 侧再次抓取第三方页面。
- 模型可由用户选择；从右臂未使用列做经典/纤细提示，但提示不覆盖用户选择。

本机皮肤库位于配置目录 `skins/library/`：

- 以规范化上传字节的 SHA-256 作为 ID，重复导入不会产生重复条目；
- 保存 PNG、合成头像和元数据索引；索引损坏或版本更高时拒绝覆盖；
- 最近使用列表按导入时间倒序；“使用”会重新校验 PNG 后再进入候选预览。
- MCP `skins_list`、`skins_import`、`skins_rename` 和 `skins_delete` 复用同一来源校验、内容 ID、索引和头像缓存；皮肤库根目录另用 `.rmcl.lock` 协调跨进程写入。它们不改变账户激活皮肤，也不包含令牌。

3D 预览由 `rmcl-app` 的 CPU 软光栅渲染器生成，支持现代 64×64、旧版 64×32、经典/纤细手臂和 HD 纹理缩放。渲染器不依赖 GPUI；桌面层把渲染放到应用层 Tokio runtime 上异步执行，只显示最新完成的 RGBA 帧，因此拖动和自动旋转不会阻塞 UI 线程。

## 错误与界面语义

- 登录代码过期 / 拒绝 / 取消：登录对话框内显示，可重试。
- 无 Minecraft 所有权、无 profile、Xbox 家庭组或地区限制：阻止添加账户。
- 启动时凭据缺失或 refresh token 失效：提示重新登录，不启动。
- 钥匙串不可用：登录、启动和删除都失败并保留现有状态。
- 换肤 401/403 且不是账号限制：提示重新登录，不静默回退离线。
- 换肤 400：显示官方返回的拒绝原因，保留候选皮肤供用户修改或取消。
- 换肤 429：显示冷却，不在后台自动重试；冷却期间禁用应用按钮。
- 换肤请求超时或连接中断：提示结果未知，要求先刷新当前皮肤确认，避免盲目重传。
- 图片直链读取失败：在链接表单内显示原因，并保留本地文件导入路径。
- 正常登录成功和正常删除账户使用一次成功通知；后台皮肤刷新失败不打扰用户。

## 配置元数据

```toml
schema_version = 2
selected_account = "Podwarden"
selected_account_kind = "microsoft"

[[microsoft_accounts]]
username = "Podwarden"
uuid = "97fe2bde-352d-454d-adf9-a0f568a50a3b"
skin_url = "http://textures.minecraft.net/texture/…"
```

`selected_account_kind` 区分同名离线账户和正版账户。离线账户仍保存在 `[[accounts]]`，正版账户保存在 `[[microsoft_accounts]]`。令牌不在其中。完整 schema 见 [启动器配置](CONFIGURATION.md)。

## 当前不包含

- authlib-injector 和第三方 Yggdrasil 登录。
- 自有 Azure 应用注册和 localhost 授权码回调。
- 皮肤站网页解析、站点专用适配和任何第三方上传。
- 披风切换/隐藏；当前只展示激活披风名称。
- 官方 owned skins 的按 ID 一键切换；是否有稳定的 URL 激活语义仍需真实账户验证。
- 皮肤编辑器、皮肤市场、HD 皮肤上传。
- 多账户批量管理、导入导出和 HMCL 账户迁移。
- Windows 和 Linux 的真实 Keychain / Credential Manager / Secret Service 验收。

默认设备代码客户端是 Microsoft 的公开 Xbox Live 客户端，便于当前无服务器分发；正式发行前应评估自有 Azure 应用注册和 `aka.ms/mce-reviewappid` 授权流程。该默认值可替换，协议层已经支持注入 client ID 和端点。

## 验证

- `rmcl-auth` 的本地 mock server 测试覆盖设备代码请求、轮询、refresh 轮换、Xbox/XSTS/Minecraft 交换、所有权检查、profile/皮肤解析、Xbox 错误、取消和秘密脱敏。
- `rmcl-app` 覆盖缓存 token 直接启动、凭据缺失、钥匙串读写/删除、schema 拒绝、官方皮肤下载、头像合成和缓存复用；新增皮肤来源校验、图片直链下载、私网地址拒绝、本地皮肤库、CPU 预览渲染、上传/reset 用例和令牌 401 重试测试。
- `rmcl-mcp` 覆盖本地库导入、重复内容去重、非法尺寸、私网 HTTPS 拒绝、重命名、删除和 read_only 写入拒绝；不访问真实皮肤站或账号。
- 2026-10-25 在 macOS ARM64 使用真实 Microsoft 账户完成设备代码登录，Keychain 写入成功，profile 与官方皮肤头像缓存成功生成。
- 皮肤上传、恢复默认、429 冷却、图片直链导入和真实窗口视觉验收仍需使用真实正版账户逐平台手动验证；自动化测试不调用真实换肤接口。
- 使用正版 Minecraft access token 的真实 Vanilla 启动已由用户在 macOS ARM64 实际验证，游戏可以正常进入运行阶段。
- Windows 和 Linux 的凭据存储与正版启动仍需平台运行测试。

协议依据：

- [Microsoft identity platform device authorization grant](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)
- [Minecraft Wiki: Microsoft authentication](https://minecraft.wiki/w/Microsoft_authentication)
- [Minecraft Wiki: Skin](https://minecraft.wiki/w/Skin)
- [Minecraft Wiki: Mojang API](https://minecraft.wiki/w/Mojang_API)（profile、upload/change/reset skin，2026-10-27 查阅）
