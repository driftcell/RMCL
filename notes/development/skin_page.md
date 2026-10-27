# 官方皮肤页与 3D 预览实现记录

日期：2026-10-27

状态：协议、应用层、3D 渲染器和页面已实现并通过自动化测试；真实换肤和窗口视觉验收待用户手动完成

## 范围

- 只支持当前选中的 Microsoft 正版账户；离线账户和第三方 Yggdrasil 不在本期范围。
- 支持本地 PNG、HTTPS 图片直链、经典/纤细模型、恢复默认皮肤和本地最近使用库。
- 不解析皮肤站网页，不做站点专用适配；只接受返回 PNG 的图片直链。
- 披风只展示激活名称，不做切换；不实现皮肤编辑器、市场或 HD 上传。

## 官方接口调研

查阅 [Minecraft Wiki: Mojang API](https://minecraft.wiki/w/Mojang_API)（2026-10-27）：

- `GET /minecraft/profile` 返回全部 owned skins（`ACTIVE` / `INACTIVE`、URL、variant、alias）和 owned capes。
- `POST /minecraft/profile/skins` 支持 multipart `variant + file` 和 JSON `variant + url`。
- `DELETE /minecraft/profile/skins/active` 恢复默认皮肤。
- 没有文档化的“按 skin id 激活”接口；owned skins 的 URL 激活是否稳定仍待真实账户验证。
- 频繁 429 可能导致账户临时限制，因此换肤不能自动重试。

尚未用真实账户确认：

- 官方当前实际大小限制（实现按历史 24 KiB 预检，超过时尝试重新编码）。
- 429 的冷却时长和 `Retry-After` 行为。
- URL 激活 owned skin 是否会新增重复条目。
- 重置接口在真实账户上是否稳定返回完整 profile。

## 分层

- `rmcl-auth`：新增 `SkinVariant`、`ProfileSkin`、`ProfileCape`，解析全部 owned skins/capes；新增 upload、URL change、reset 方法和可注入端点。
- `rmcl-app/skin_source`：本地文件和图片直链的统一校验、下载、尺寸/大小检查和上传字节规范化。
- `rmcl-app/skin_library`：以规范化上传字节 SHA-256 为 ID 的本地皮肤库，保存 PNG、合成头像和 JSON 索引。
- `rmcl-app/skin_preview`：无 GPUI 依赖的 CPU 软光栅 3D 预览，支持现代/旧版、经典/纤细、HD 缩放和半透明外层。
- `apps/rmcl/skin_service`：按账户缓存 profile 和纹理，任务跟随 loading 状态，使用请求序号拒绝过期结果。
- `apps/rmcl/skin_viewer`：独立 GPUI Entity，只重绘预览区；用 `request_animation_frame` 推进自动旋转。
- `apps/rmcl/ui/skin`：页面组合、候选皮肤、链接导入、应用/冷却状态和最近使用列表。

## 状态取舍

没有把整个页面做成一个大 enum。可以同时发生的部分用 struct 组合，互斥部分用小 enum：

- `ProfileState`：loading / loaded / failed，刷新时保留上一份 profile。
- `EquippedSkinState`：loading / ready / failed，按 URL 判断是否需要重新下载。
- `Loadable<T>`：最近使用列表等通用异步数据。
- `ApplyState`：idle / applying / cooldown，Task 放在变体里。
- `LinkImportState`：idle / loading / failed。
- `SkinEditor`：候选 PNG 和模型选择。
- `SkinViewer`：yaw/pitch、旋转模式、渲染缓存。

## 渲染

- 6 个长方体部件，z-buffer、透视投影、perspective-correct UV、逐面光照、最近邻采样。
- 2× 超采样后按预乘 alpha 下采样，透明背景边缘不产生黑边。
- 旧版 64×32 左肢使用右侧纹理并交换左右侧面 UV。
- 半透明外层按远到近排序，四边形共享边分配给其中一个三角形，避免对角线重复混合。
- 产能未做 release 基准；页面视觉和帧率由用户在真实窗口验收。

## 配置与路径

- 当前激活皮肤 URL 仍只保存在 `microsoft_accounts[].skin_url`，配置 schema 不变。
- 完整纹理缓存为 `cache/skins/<sha1(url)>.skin.png`，头像仍为 `cache/skins/<sha1(url)>.png`。
- 本地皮肤库为 `skins/library/`，索引 `index.json`，条目文件 `<sha256>.png` 和 `<sha256>.head.png`。

## 后续

- 用真实正版账户验证上传、重置、429、图片直链和 owned skins URL 激活。
- 在 macOS、Windows、Linux 真实窗口检查布局、拖拽、动画、主题和缩放。
- 根据实际皮肤/材质决定是否增加披风渲染和更高质量的预览。
