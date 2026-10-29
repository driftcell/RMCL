# authlib-injector 外置登录

状态：协议、账户、启动注入、下载缓存、桌面账户流程和 MCP 启动已实现；真实第三方服务端验收待完成

日期：2026-10-29

## 范围

RMCL 支持通过 authlib-injector 使用第三方 Yggdrasil 认证服务器：

- 可同时保存多个认证服务器上的账户。
- 内置常用公共认证服务器，同时保留“自定义服务器”入口。
- 账户密码只用于登录请求，不写入配置、日志或系统凭据存储。
- 访问令牌与客户端令牌保存在系统凭据存储。
- 启动前校验令牌，失效时刷新；刷新失败要求重新登录，不静默回退离线身份。
- 运行期下载 authlib-injector，校验 SHA-256 并缓存。
- 游戏内皮肤、披风和聊天签名由 authlib-injector 按认证服务器元数据处理。

RMCL 不解析第三方皮肤站网页，也不提供第三方皮肤上传或披风切换。认证服务器必须实现 Yggdrasil API；仅支持网页 OAuth、扫码或社交账号登录的站点不在当前范围内。

## 内置服务器

以下地址在 2026-10-29 核对。用户仍可通过自定义服务器填写其他 API Root。

| 名称 | API Root | 主页 |
|---|---|---|
| LittleSkin | `https://littleskin.cn/api/yggdrasil/` | `https://littleskin.cn/` |
| 红石皮肤站 | `https://mcskin.cn/api/yggdrasil/` | `https://mcskin.cn/` |
| HelloSkin | `https://helloskin.cn/api/yggdrasil/` | `https://helloskin.cn/` |
| MUA 用户中心 | `https://skin.mualliance.ltd/api/yggdrasil/` | `https://skin.mualliance.ltd/` |
| Ely.by | `https://account.ely.by/api/authlib-injector/` | `https://ely.by/` |

内置列表只是便捷入口，不代表 RMCL 与这些服务存在隶属或担保关系。服务可用性、条款和账号政策由各站点负责。

## API Root 解析

`rmcl-auth` 实现 authlib-injector 启动器技术规范中的 API 地址指示（ALI）：

1. 用户输入缺少协议时补 `https://`。
2. 请求输入地址，读取 `X-Authlib-Injector-API-Location`。
3. 支持绝对和相对 ALI，最多跟随 5 次，检测循环。
4. 最终地址规范化并补尾斜杠，保存为账户的 API Root。
5. 显式 HTTP 输入允许使用，便于自建或本地服务；但 HTTPS 不会因为重定向或 ALI 自动降级到 HTTP。
6. 认证 POST 不跟随重定向，避免密码或令牌被转发到其他来源。

服务器元数据保留原始响应字节，用于 authlib-injector 的 prefetch。元数据响应上限 256 KiB，认证响应上限 1 MiB。

## 登录与令牌

使用标准 Yggdrasil 端点：

- `POST {api_root}/authserver/authenticate`
- `POST {api_root}/authserver/validate`
- `POST {api_root}/authserver/refresh`
- `POST {api_root}/authserver/invalidate`

登录请求包含 `agent = { name: "Minecraft", version: 1 }`、`requestUser = true` 和随机 `clientToken`。响应中的 `clientToken` 必须与请求一致。

账户可能有多个角色。若服务器没有返回 `selectedProfile` 且存在多个 `availableProfiles`，桌面端要求用户选择角色，再通过 refresh 绑定该角色。刷新后的角色 UUID 必须与保存的账户一致。

系统凭据存储：

- 服务名：`com.rmcl.launcher`
- 键：`authlib:<RMCL 账户 ID>`
- 值：版本化 JSON，包含 `access_token`、`client_token` 和 `user_properties`

RMCL 账户 ID 在首次登录时生成并保存在 `config.toml`，玩家改名不会改变它。不同认证服务器即使使用相同玩家名或 UUID，也通过账户 ID 区分。

## 启动注入

authlib-injector 作为 Java Agent 加载，启动计划加入：

```text
-javaagent:<jar>=<api_root>
-Dauthlibinjector.side=client
-Dauthlibinjector.yggdrasil.prefetched=<Base64(原始元数据)>
```

这些参数位于主类之前。账户启动身份使用 `user_type = "msa"`，与 HMCL 固定基线一致；authlib-injector 也会把 `mojang` 规范化为 `msa`。`user_properties` 使用 Yggdrasil 登录响应中的用户属性，缺失时为 `{}`。元数据 Base64 超过 24 KiB 时省略 prefetch，改由 agent 自行请求，避免 Windows 命令行长度限制。

启动前流程：

1. 从系统凭据存储读取令牌。
2. 请求 API Root 获取最新元数据；网络失败时使用磁盘元数据缓存。
3. 调用 validate。
4. 令牌有效则直接启动。
5. 令牌失效则 refresh 并写回新令牌。
6. 获取或复用已校验的 authlib-injector jar。
7. 构造启动计划并启动游戏。

令牌、客户端令牌和密码不会进入启动计划诊断输出；`LaunchPlan` 的脱敏逻辑覆盖 access token。

## authlib-injector 下载

官方下载 API：

```text
https://authlib-injector.yushi.moe/artifact/latest.json
```

BMCLAPI 镜像：

```text
https://bmclapi2.bangbang93.com/mirrors/authlib-injector/artifact/latest.json
```

RMCL 使用现有下载源偏好选择候选源，读取 `download_url` 和 `checksums.sha256`，下载后校验 SHA-256，再原子发布到配置目录的 `cache/authlib-injector/`。缓存按 build number 保存 jar 和校验元数据；网络不可用时使用最新通过校验的缓存。

authlib-injector 使用 AGPL-3.0 及 “AUTHLIB-INJECTOR” 例外。RMCL 当前不随安装包分发该 jar，而是在运行期下载；如果未来改为打包，必须保留未修改二进制、许可证文本和来源说明。

## 皮肤语义

游戏内皮肤由 authlib-injector 替换认证与材质域名白名单后自动加载。RMCL 不下载第三方皮肤作为账户头像，也不提供第三方皮肤上传、切换或披风管理。

桌面皮肤页对当前选中的外置账户显示说明状态，不会把它当作 Microsoft 账户处理。后续若要支持第三方头像，必须使用元数据 `skinDomains` 做域名白名单校验，并避免不同服务器同 UUID 的缓存冲突。

## 验证

自动化测试覆盖：

- `rmcl-auth`：URL 规范化、显式 HTTP、ALI、元数据、登录、错误密码、validate、refresh、clientToken 校验和秘密脱敏。
- `rmcl-store`：schema 8 → 9、外置账户往返、稳定账户 ID 和选中项迁移。
- `rmcl-app`：登录、角色选择、令牌刷新、凭据删除、元数据缓存、jar 下载、SHA-256 校验、缓存复用和离线回退。
- `rmcl-launch`：`-javaagent`、`side=client`、prefetch、`user_type = msa`、缺失 jar 拒绝和 token 脱敏。
- `rmcl-mcp`：外置账户列表、选择、启动注入和删除凭据。

2026-10-29 核对了五个内置 API Root 的元数据端点，均返回 HTTP 200 和 JSON；官方与 BMCLAPI 的 authlib-injector manifest 均为 build 56 / 1.2.8，官方 jar 下载后的 SHA-256 与 manifest 一致。

真实第三方服务端登录、真实游戏进服、皮肤显示和聊天签名仍需在 Windows、Linux 和 macOS 分别手工验收；CI 不访问真实账号或第三方认证服务。
