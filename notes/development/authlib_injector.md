# authlib-injector 外置登录实现记录

日期：2026-10-29

状态：协议、账户、启动注入、下载缓存、桌面流程和 MCP 启动已实现；真实第三方服务端验收待完成

## 目标与边界

实现多个第三方 Yggdrasil 认证服务器账户，使用 authlib-injector 在游戏内替换认证与材质端点。用户确认的取舍：

1. 支持多个认证服务器。
2. 引入稳定 `selected_account_id`。
3. authlib-injector 运行期下载，SHA-256 校验并缓存。
4. 游戏内皮肤由 authlib-injector 处理。
5. `user_type` 使用 `msa`。
6. 允许显式 HTTP，同时内置常见公共认证服务器，不把填写 URL 作为唯一入口。

不包含第三方皮肤上传、披风切换、皮肤站网页解析、Yggdrasil Connect/OAuth 登录和密码通过 MCP 传输。

## 调研来源

- authlib-injector 启动器技术规范：<https://github.com/yushijinhun/authlib-injector/wiki/%E5%90%AF%E5%8A%A8%E5%99%A8%E6%8A%80%E6%9C%AF%E8%A7%84%E8%8C%83>（2026-10-29 查阅）
- authlib-injector 下载 API：<https://yushijinhun.github.io/authlib-injector/en/get-authlib-injector.html>（2026-10-29 查阅）
- LittleSkin Yggdrasil 文档：<https://manual.littleskin.cn/yggdrasil/>（2026-10-29 查阅）
- HMCL 固定基线 `0c3c7dd62ab073bf384fd0f57914388aae1359c4` 的 authlib-injector 账户、启动参数和元数据缓存行为。
- 公共认证服务器地址来自 LittleSkin、红石皮肤站、HelloSkin、MUA 用户中心和 Ely.by 的公开文档；内置列表是便捷入口，不代表担保。

## 关键决策

### 账户身份

Yggdrasil profile UUID 只在单个认证服务器内有意义，不同服务器可能复用相同 UUID 或玩家名。RMCL 在首次完成登录时生成随机账户 ID，保存在 `authlib_accounts[].id`，并作为：

- `selected_account_id = authlib:<id>`
- 系统凭据键 `authlib:<id>`
- UI 账户稳定键

玩家改名不会改变账户 ID。相同服务器、相同 profile 再次登录时复用已有账户 ID。

### API Root 与 HTTP

用户输入缺少协议时补 HTTPS；支持 ALI 相对/绝对跳转和循环检测。显式 HTTP 输入允许使用，便于自建或本地服务；但 HTTPS 不允许因重定向或 ALI 自动降级到 HTTP。认证 POST 不跟随重定向，避免密码或令牌被转发。

### 令牌生命周期

登录使用随机 clientToken，响应 clientToken 必须一致。启动前 validate；失效时 refresh，并验证 selectedProfile UUID 未改变。refresh 失败要求重新登录，不回退离线身份。密码只存在于一次登录请求中。

### 启动注入

`LaunchOptions.authlib` 携带 jar 路径、API Root 和 Base64 原始元数据。启动计划加入：

```text
-javaagent:<jar>=<api_root>
-Dauthlibinjector.side=client
-Dauthlibinjector.yggdrasil.prefetched=<Base64(原始元数据)>
```

使用 `user_type = "msa"`，与 HMCL 基线一致。authlib-injector 的 `AccountTypeTransformer` 也会把规范中的 `mojang` 转为 `msa`。

### 下载与缓存

官方 manifest 和 BMCLAPI 镜像通过现有 `MirrorHealth` 选择。下载后校验 SHA-256，按 build number 保存 jar 和校验元数据。每 24 小时检查一次最新 manifest；网络失败时使用最新通过校验的缓存。当前不把 jar 打入安装包。

### 内置服务器

桌面端提供 LittleSkin、红石皮肤站、HelloSkin、MUA 用户中心和 Ely.by 的下拉选项，同时保留“自定义服务器”。不硬编码为唯一服务器，避免私有服和自建服无法使用。

## 实现位置

- `crates/rmcl-auth/src/authlib.rs`：协议、ALI、元数据、账户、凭据、预设和测试。
- `crates/rmcl-auth/src/account.rs`：离线/Microsoft/Authlib 统一账户枚举。
- `crates/rmcl-store/src/config.rs`：schema 9、`authlib_accounts`、`selected_account_id` 和迁移。
- `crates/rmcl-app/src/authlib_accounts.rs`：登录、角色选择、刷新、凭据存储和元数据缓存。
- `crates/rmcl-app/src/authlib_injector.rs`：manifest、下载、SHA-256、缓存和离线回退。
- `crates/rmcl-launch/src/account.rs`、`options.rs`、`plan.rs`：启动身份和 JVM 参数。
- `apps/rmcl/src/app/mod.rs`：外置登录对话框、角色选择、账户列表、切换和删除。
- `apps/rmcl/src/app/ui/skin.rs`：外置账户皮肤页说明状态。
- `crates/rmcl-mcp/src/server.rs`、`schema.rs`：外置账户列表、选择、启动注入和删除。

## 验证

- `rmcl-auth`、`rmcl-store`、`rmcl-app`、`rmcl-mcp` 和 `rmcl-launch` 的 authlib 相关测试全部通过。
- 全 workspace `cargo nextest run --workspace --locked`：506 个测试通过，9 个网络 smoke 标记 skipped。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 和 `cargo test --workspace --doc --locked` 通过。

2026-10-29 额外核对了内置服务器的 API Root 元数据端点，均返回 HTTP 200 和 JSON；官方与 BMCLAPI 的 authlib-injector manifest 均为 build 56 / 1.2.8，官方 jar 下载后的 SHA-256 与 manifest 一致（`9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10`）。

真实第三方服务端登录、进服、皮肤和聊天签名尚未验收。CI 不访问真实账号或第三方认证服务。
