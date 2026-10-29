# Microsoft 登录与正版皮肤实现记录

日期：2026-10-25

状态：核心与 macOS 实际登录已验证

## 目标

实现 Microsoft 正版账户登录、令牌刷新、账户切换/删除和官方皮肤头像；不接入第三方皮肤站、authlib-injector 或披风。

## 流程选择

比较了三条路线：

1. 内嵌 WebView + 授权码回调：体验接近 HMCL，但引入平台 WebView、窗口和打包复杂度。
2. 系统浏览器 + 自有 Azure 应用 + localhost 回调：体验顺滑，但需要 Azure 应用注册、redirect URI 和 Minecraft API 授权。
3. 系统浏览器 + 设备代码：无需客户端密钥或自有应用注册，能立即实现和验证，代价是用户需要输入一次性代码。

本次选择设备代码，保留 `MicrosoftEndpoints` 注入能力，后续可以替换为自有 Azure 应用和 localhost 回调。

默认客户端使用 Microsoft 公开 Xbox Live 客户端 `00000000402b5328`，通过 `login.live.com` 的设备代码端点工作。正式发行前需要评估自有应用注册和 `aka.ms/mce-reviewappid` 授权，不能把共享客户端视为长期产品方案。

## 协议与状态

实现位置：`crates/rmcl-auth/src/microsoft.rs`。

- 设备代码请求、轮询、`slow_down`、过期、拒绝、取消和临时错误重试。
- Microsoft token → Xbox Live → XSTS → Minecraft services。
- Java 版所有权检查，要求 `game_minecraft` 或 `product_minecraft`。
- profile UUID、玩家名和激活皮肤 URL 解析。
- refresh token 轮换；服务端未返回新 token 时保留旧值。
- Minecraft token 过期前 5 分钟刷新。
- 所有含 token 的类型使用自定义 `Debug` 脱敏。

错误保留 OAuth code、Xbox `XErr`、HTTP 状态和操作名，界面据此给出中文提示；核心错误文本保持英文。

## 令牌存储

比较了独立凭据文件和系统钥匙串。选择钥匙串：

- `rmcl-platform` 定义 `SecretStore` trait，生产使用 `KeyringSecretStore`。
- 服务名 `com.rmcl.launcher`，账户键 `microsoft:<无连字符 UUID>`。
- 只保存 refresh token、缓存的 Minecraft access token 和绝对过期时间；Microsoft access token 不落盘。
- `config.toml` 只保存玩家名、profile UUID 和皮肤 URL。
- 钥匙串测试使用内存实现；macOS 真实 Keychain 通过 ignored 测试验证。
- 删除账户先删凭据再删配置，失败时保留账户并提示。

## 配置与账户模型

配置升级到 schema 2：

- 新增 `selected_account_kind`、`microsoft_accounts` 和 `skin_url`。
- schema 1 读取时默认选中离线账户，其他新字段为空；读取不重写文件。
- 账户下拉使用 `AccountSelectItem` 的稳定键，显示名不再附加“（正版）”。
- 同名离线/正版账户可以共存。

删除账户在账户列表每行提供入口，确认后执行。删除当前账户会自动选择下一个可用账户；删除最后一个账户后主页回到“添加账户”状态。

## 官方皮肤头像

`rmcl-app/src/skins.rs`：

- 只接受 `textures.minecraft.net`（含重定向后的最终主机）。
- HTTP/HTTPS、最多 1 MiB、只接受 PNG，宽度至少 64 且为 64 的倍数，高度至少 32。
- 基础面部 `(8,8)` + 帽子层 `(40,8)` alpha 合成，最近邻放大到 64×64。
- 缓存到配置目录 `cache/skins/<sha1(url)>.png`，临时文件原子发布。
- 旧账户缺少 `skin_url` 时，启动后台用缓存 token 刷新 profile；失败静默回退首字母。
- 侧栏、账户列表和下拉显示头像；离线账户显示首字母。

## UI 取舍

- 登录对话框：紧凑代码卡片、内嵌复制反馈、整行“打开浏览器”、轻量取消。
- 复制后按钮切换为绿色圆形对勾，固定 24px，不引起布局变化。
- 账户对话框列出离线/正版账户，支持添加、登录和删除；删除确认叠加在原对话框上。
- 主页快速操作只保留安装新版本和打开游戏目录，管理账户通过侧栏账户卡片进入。
- 启动正版账户时在主按钮显示“正在验证账户…”，解析完成后直接进入安装/启动任务。

## 验证

- `rmcl-auth`：20 个测试，本地 mock server 校验请求体和响应解析。
- `rmcl-app`：皮肤下载/合成/缓存、Keychain 用例、schema 错误、凭据生命周期。
- `rmcl-platform`：内存存储测试；macOS Keychain ignored 测试手动通过。
- 真实官方皮肤纹理 ignored 测试通过。
- 2026-10-25 macOS ARM64：真实 Microsoft 账户设备代码登录成功，Keychain 写入成功，profile 与皮肤头像缓存生成。
- 使用正版 access token 的真实 Vanilla 启动已由用户实际验证，游戏正常进入运行阶段。
- 全 workspace：231 个 nextest 测试通过。

## 未决

- Windows Credential Manager、Linux Secret Service 和这两个平台上的正版启动未运行验证。
- 披风、皮肤上传和自有 Azure 应用注册未实现。authlib-injector 外置登录已于 2026-10-29 另行实现，见 [authlib-injector 外置登录实现记录](authlib_injector.md)。
- 公开客户端 ID 的长期可用性和发行合规需要后续评估。
