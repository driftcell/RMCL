# 正版账号、令牌与皮肤

状态：Microsoft 设备代码登录、令牌刷新、账户删除和官方皮肤头像已实现

日期：2026-10-25

## 边界

正版登录按职责拆分：

- `rmcl-auth`：无 UI 的 Microsoft/Xbox/Minecraft 协议、设备代码流程、令牌刷新、账户身份与错误类型。
- `rmcl-platform`：操作系统凭据存储的通用 `SecretStore` 接口和 Keychain / Credential Manager / Secret Service 适配。
- `rmcl-store`：`config.toml` 中的账户元数据，不保存任何令牌。
- `rmcl-app`：登录与启动所需的用例编排、凭据读写、皮肤下载与头像缓存。
- `rmcl-launch`：消费已解析的 `LaunchAccount`，只负责生成启动参数和进程。
- `apps/rmcl`：GPUI 对话框、账户列表、删除确认和头像展示。

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

## 错误与界面语义

- 登录代码过期 / 拒绝 / 取消：登录对话框内显示，可重试。
- 无 Minecraft 所有权、无 profile、Xbox 家庭组或地区限制：阻止添加账户。
- 启动时凭据缺失或 refresh token 失效：提示重新登录，不启动。
- 钥匙串不可用：登录、启动和删除都失败并保留现有状态。
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
- 披风展示、皮肤上传、皮肤库和第三人称全身预览。
- 多账户批量管理、导入导出和 HMCL 账户迁移。
- Windows 和 Linux 的真实 Keychain / Credential Manager / Secret Service 验收。

默认设备代码客户端是 Microsoft 的公开 Xbox Live 客户端，便于当前无服务器分发；正式发行前应评估自有 Azure 应用注册和 `aka.ms/mce-reviewappid` 授权流程。该默认值可替换，协议层已经支持注入 client ID 和端点。

## 验证

- `rmcl-auth` 的本地 mock server 测试覆盖设备代码请求、轮询、refresh 轮换、Xbox/XSTS/Minecraft 交换、所有权检查、profile/皮肤解析、Xbox 错误、取消和秘密脱敏。
- `rmcl-app` 覆盖缓存 token 直接启动、凭据缺失、钥匙串读写/删除、schema 拒绝、官方皮肤下载、头像合成和缓存复用；另有手动 `--ignored` 测试访问真实官方皮肤纹理。
- 2026-10-25 在 macOS ARM64 使用真实 Microsoft 账户完成设备代码登录，Keychain 写入成功，profile 与官方皮肤头像缓存成功生成。
- 真实游戏启动使用正版 token 仍待单独验收；Windows 和 Linux 的凭据存储需要平台运行测试。

协议依据：

- [Microsoft identity platform device authorization grant](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)
- [Minecraft Wiki: Microsoft authentication](https://minecraft.wiki/w/Microsoft_authentication)
- [Minecraft Wiki: Skin](https://minecraft.wiki/w/Skin)
