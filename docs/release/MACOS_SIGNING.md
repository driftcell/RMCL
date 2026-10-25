# macOS 签名与公证

状态：发布 workflow 已支持；需要仓库维护者提供 Apple Developer 凭据

日期：2026-10-25

## 为什么需要

macOS Gatekeeper 会隔离从浏览器下载的应用。ad-hoc 签名只能保证二进制在 Apple Silicon 上可执行，不能让 Gatekeeper 把应用识别为可信发行者；未公证的 `.app` 可能直接提示“已损坏”或无法打开。

正式分发需要：

1. 加入 Apple Developer Program；
2. 创建 **Developer ID Application** 证书；
3. 使用 Developer ID 签名并启用 hardened runtime；
4. 提交 Apple 公证（notarization）；
5. 将公证票据 staple 到 `.app` 后再打包。

免费 Apple ID、Apple Development 证书和自签名证书都不能替代 Developer ID + 公证。

## 准备凭据

### 1. 加入 Apple Developer Program

个人账号使用 Apple ID 注册并完成身份验证；公司账号需要 D-U-N-S 编号。会员按年收费，价格以 Apple 当前地区政策为准。

### 2. 创建 Developer ID Application 证书

1. 打开 Keychain Access → Certificate Assistant → Request a Certificate from a Certificate Authority。
2. 选择保存到磁盘，生成 `.certSigningRequest`。
3. 在 Apple Developer 后台 Certificates 中新建 **Developer ID Application** 证书，上传 CSR。
4. 下载 `.cer`，双击安装到 Keychain。
5. 在 Keychain 中选中证书和私钥，导出 `.p12` 并设置密码。
6. 记录 Team ID（10 位）和证书的完整名称，例如 `Developer ID Application: Name (TEAMID)`。

不要提交 `.p12`、密码或 app-specific password 到仓库。

### 3. 创建 App-Specific Password

在 Apple ID 账户安全设置中生成 app-specific password，用于 `xcrun notarytool`。也可以改用 App Store Connect API key，workflow 当前使用 Apple ID 方式。

## GitHub Secrets

在仓库 Settings → Secrets and variables → Actions 中配置：

| Secret | 值 |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | `.p12` 文件的 base64：`base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 时设置的密码 |
| `APPLE_SIGNING_IDENTITY` | 证书完整名称 |
| `APPLE_ID` | Apple Developer 账户邮箱 |
| `APPLE_APP_SPECIFIC_PASSWORD` | Apple ID app-specific password |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

## 发布流程行为

`.github/workflows/release.yml` 的 macOS job：

- 所有 secrets 存在时：
  - 在临时 Keychain 中导入证书；
  - 对主二进制和 `.app` 使用 `codesign --options runtime --timestamp` 签名；
  - 用 `ditto` 生成公证提交包；
  - `xcrun notarytool submit --wait`；
  - `xcrun stapler staple` 和 `xcrun stapler validate`；
  - 打包最终 `.app`，job 结束时删除临时 Keychain。
- 缺少任一 secret 时：
  - 打印 warning；
  - 回退到 ad-hoc 签名；
  - 下载后仍需用户手动移除 quarantine。
- 证书导入或公证失败时：
  - job 失败，不创建 Release；不会静默发布未签名包。

## 验证签名产物

```bash
codesign --verify --deep --strict --verbose=2 RMCL.app
codesign -dv --verbose=4 RMCL.app
spctl --assess --type execute --verbose=4 RMCL.app
xcrun stapler validate RMCL.app
```

成功时 `spctl` 应报告 `accepted`，`stapler` 应报告票据有效。仅在本机签名成功不代表公证通过，必须检查 `notarytool` 结果。

## 没有 Developer ID 时

只能使用 ad-hoc 签名，并让用户手动移除隔离属性：

```bash
xattr -dr com.apple.quarantine RMCL.app
open RMCL.app
```

这要求用户信任构建来源，不应把这种方式描述为正常的 macOS 分发体验。也可以让用户从源码构建，避免下载隔离。

## 维护事项

- Developer ID 证书和 app-specific password 需要定期轮换；证书到期前更新 secret。
- 不在 issue、日志或截图中暴露证书、Team ID 以外的账号凭据。
- 公证和签名需要联网；GitHub-hosted macOS runner 必须能访问 Apple 服务。
- 未来可以增加 DMG、公证日志归档和 App Store Connect API key 支持。
