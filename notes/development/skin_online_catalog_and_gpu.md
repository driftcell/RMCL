# 在线皮肤目录与 GPU 预览调研

日期：2026-11-01

状态：LittleSkin 在线目录、本地 3D 缩略图、候选确认和正版账户一键应用已实现（2026-10-31）；2026-11-01 修复筛选空页分页、失败选中态并取消过期目录请求；GPU 预览仍停留在方案评估，未实施

## 已实现范围

- `crates/rmcl-app/src/skin_catalog.rs`：LittleSkin 列表、搜索、热门/最新排序、分页、详情 hash 解析和纹理准备；只保留 public、`steve`/`alex`、非 HD 条目。
- `apps/rmcl/src/app/skin_service.rs`：目录加载、缩略图批量任务、选择态、纹理下载和 GPUI `RenderImage` 缓存；列表、缩略图和选择请求持有可取消 token，缩略图共用一个 4 并发上限。
- `apps/rmcl/src/app/ui/skin.rs`：在线/本机双 tab、双列缩略图卡片、搜索/排序/加载更多、来源与外链、选中态和底部确认栏；筛选后整页为空时仍可加载下一页，失败卡片不显示选中态，离线/外置/未选择账户可预览但不可应用。
- 缩略图由本地 `SkinPreviewRenderer` 生成 180×240 RGBA，不依赖 LittleSkin 的预览图；选中后左侧舞台继续使用实时 3D 预览。
- 仍未做 GPU 渲染、MineSkin 目录、自定义站点、第三方皮肤上传和 HD 皮肤上传。

## 结论

- 在线皮肤浏览在技术上可行，但没有 Mojang 官方皮肤市场或官方目录 API。
- 目前可直接使用的公开接口只有两类：
  - MineSkin V2：有文档，可列出最近生成的公开皮肤，但没有搜索、分类和作者信息。
  - Blessing Skin 系站点（如 LittleSkin）：站点前端使用的 JSON 接口可用，但不是稳定公开 API，需要按站点做兼容适配。
- NameMC、SkinMC、The Skindex、mcskindb 等没有可依赖的公开目录 API；网页抓取不稳定，且部分站点明确禁止下载路径抓取，不应作为产品依赖。
- 一键换肤只对 Microsoft 正版账户成立：下载 PNG、校验、再走现有 `POST /minecraft/profile/skins` 上传。离线账户和 authlib 外置账户当前没有换肤路径。
- 不建议为了 432×620 的皮肤预览直接改 GPUI 渲染后端。现有 CPU 光栅器在本机一次测试中只需约 1.75 ms/帧（交互帧），不是当前 CPU 占用的主要来源。应先做帧调度和纹理上传的测量与修正，再决定是否引入 GPU。

## 在线目录 API 核实

### MineSkin V2

文档：<https://docs.mineskin.org/docs/mineskin-api/get-a-list-of-skins>

- `GET https://api.mineskin.org/v2/skins`：返回最近的公开皮肤列表，字段包括 `uuid`、`shortId`、`name`、`texture`、`timestamp`，用 `after` 做游标分页。
- `GET https://api.mineskin.org/v2/skins/{uuid}`：返回完整皮肤信息，包括 `variant`、`visibility`、`texture.url.skin`。
- 匿名请求可以读取，但会返回 `no_api_key` 警告；官方建议设置 `User-Agent` 并使用 API Key。匿名配额和可见范围不能当作稳定契约。
- 列表没有搜索、标签或作者字段，`name` 也经常为空；它更像“最近生成”流，不是可浏览的皮肤商店。
- 内容没有面向 RMCL 的审核保证，接入前必须做内容过滤、举报和隐藏能力。

### Blessing Skin / LittleSkin

参考源码：<https://github.com/bs-community/blessing-skin-server>

LittleSkin 实测（2026-10-31）：

- `GET /skinlib/list?filter=skin&sort=likes&page=1`，带 `Accept: application/json`，返回分页 JSON：`tid`、`name`、`type`（`steve` / `alex`）、`likes`、`nickname`、`hd` 等。
- `GET /skinlib/info/{tid}` 返回纹理元数据，包括 `hash`、`type`、`public`、`hd`。
- 原图地址为 `GET /textures/{hash}`，返回 `image/png`。

注意事项：

- 这是站点前端使用的接口，不是 Blessing Skin 文档化的公开 API；不同站点、插件和隐私设置会改变行为。
- 适合做“可配置的 Blessing Skin 站点适配”，不能假设所有站点都开放皮肤库。
- 有作者、点赞和模型信息，比 MineSkin 更适合做浏览界面。
- 仍需要遵守站点条款、控制请求频率，并缓存已下载纹理，不能把第三方站点当 CDN 热链。

### 其他站点

| 来源 | 可核实能力 | 结论 |
| --- | --- | --- |
| Mojang profile API | 只返回当前账户拥有的皮肤和披风 | 不是目录 |
| Ely.by | 按昵称查询皮肤/披风纹理和 profile | 不是目录 |
| Crafty public API | 按玩家名、玩家 UUID 或皮肤 hash 查询 | 有文档，但没有目录浏览端点 |
| NameMC | 有网页目录 | 没有公开文档化 API，不依赖抓取 |
| SkinMC | 有网页目录和渲染服务 | 没有公开目录 API |
| The Skindex / minecraftskins.com | 有网页目录 | `robots.txt` 禁止 `/skin/download/`、`/actions/` 等路径，不应抓取 |
| mcskindb | 有网页目录 | 没有公开 API |

## 推荐的产品形态

不要先绑定某一个皮肤站，而是在 `rmcl-app` 里定义目录提供方边界：

```text
SkinCatalogProvider
  - id / display_name / attribution_url
  - search(query, page, variant) -> CatalogPage
  - texture(item_id) -> validated PNG bytes
```

首批实际只实现 LittleSkin/Blessing Skin 适配；MineSkin 最近流和自定义站点保留为后续：

1. `LittleSkin`：使用站点 JSON 目录，有名称、作者、点赞、模型和分页。
2. `MineSkinRecent`（未实现）：作为“最近公开皮肤”流，需要内容过滤，且没有搜索和作者。
3. 自定义 Blessing Skin 站点（未实现）：需要逐站点验证接口、隐私设置和内容政策。

界面放在现有皮肤页的“在线”标签，和“本机”“链接”并列：

- 网格卡片只显示缩略图和来源；选中后才下载完整 PNG 并进入现有 `SkinViewer`。
- “应用到账户”复用现有候选、预览、上传和冷却状态；不新增第二套换肤逻辑。
- “保存到本机”写入现有 `skins/library/`，按规范化字节 SHA-256 去重。
- Microsoft 账户可用“应用”；离线账户和 authlib 账户显示明确限制，或只允许保存到本机。
- 显示来源、作者（若提供）和“在浏览器打开”；提供隐藏/举报入口。
- 不把第三方皮肤打包进安装包；不自动批量抓取；不把目录内容写入启动计划或日志。

如果要做 LittleSkin 账户的一键换肤，需要额外的 Yggdrasil 纹理上传能力和账户授权，这超出当前“外置登录只负责登录与启动注入”的边界，应单独立项。

## CPU 预览的实测

测试代码临时放在 `rmcl-app` 的 example 中，使用 64×64 合成皮肤、432×620 输出，在本机（macOS ARM64）开发配置下运行 100 帧后取平均；`rmcl-app` 本身使用 `opt-level = 2`：

| 模式 | 每帧耗时 | 折算帧率 |
| --- | ---: | ---: |
| `SkinPreviewRenderer::fast` 432×620 | 1.75 ms | ~573 fps |
| `SkinPreviewRenderer::new` 432×620（2× 超采样） | 7.48 ms | ~134 fps |
| `fast` 216×310 | 0.49 ms | ~2060 fps |

这不是正式基准，但足以说明：在自动旋转使用的 `fast` 路径下，30 fps 的 CPU 光栅只占单核约 5%，不可能单独造成明显的高 CPU。更值得检查的是：

- `apps/rmcl/src/app/skin_viewer.rs` 每 33 ms 启动一个后台任务，并每帧创建新的 `RenderImage`，随后由 GPUI 上传到 sprite atlas；旧图立即 `drop_image`，存在持续的分配、上传和图集抖动。
- `window.request_animation_frame()` 按显示器刷新率推进，画面更新却只有 30 fps；窗口不可见或未聚焦时也没有停止自动旋转。
- 如果实际运行的是未优化的 debug 依赖，或 `fast` 没有生效，耗时会被放大。
- 需要分别测量 `renderer.render`、RGBA→BGRA 转换、`RenderImage` 创建、GPUI 上传和窗口绘制，不能只按“CPU 渲染”归因。

## GPUI 内嵌 GPU 的可行路径

已核对 `gpui-pre 0.3.10`：

- `gpui::canvas` 只提供 `paint_quad`、`paint_path`、`paint_image` 等 2D 绘制命令，不能提交自定义三角形、纹理或 shader。
- `gpui::surface()` / `Window::paint_surface()` 存在，但仅限 macOS/iOS，参数是 `CVPixelBuffer`。
- `gpui-pre-apple` 的 Metal 渲染器对 surface 断言 `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`，只适合视频帧；没有 alpha，色度 4:2:0 会破坏皮肤像素边缘，不能直接用于透明背景的皮肤预览。
- `RenderImage` 是 CPU 端 BGRA 缓冲区，`paint_image` 会写入 sprite atlas；没有公开的外部纹理或共享纹理入口。

### 方案 A：GPU 离屏渲染 + 回读为 `RenderImage`（推荐先评估）

- 用 `wgpu` 在后台渲染 6 个长方体到 RGBA8 离屏纹理，带深度缓冲、最近邻采样和透明混合。
- 通过 staging buffer 异步回读，转换成 BGRA，再构造 `RenderImage`，继续用现有 `img()` 显示。
- 不需要改 GPUI，跨 macOS/Windows/Linux。
- 代价是每帧一次 GPU→CPU 回读和一次 CPU→GPU 上传；432×620 约 1 MiB/帧，60 fps 约 64 MiB/s，通常可接受，但必须做三缓冲并丢弃过期帧。
- 实现放在 `rmcl-app`，由 `SkinPreviewBackend` trait 隔离；GPUI 只消费帧，不依赖 `wgpu`。

### 方案 B：macOS 零拷贝 `CVPixelBuffer`

- 用 Metal 渲染到 IOSurface-backed `CVPixelBuffer`，再通过 `gpui::surface()` 直接采样。
- 当前 GPUI 只接受 420f YCbCr，且不支持透明；要用 BGRA，需要 fork `gpui-pre`，在 Metal 渲染器中增加 BGRA pipeline 和 shader 分支。
- 还要处理 `CVPixelBuffer` 池、跨设备和跨帧复用、GPUI 何时释放 surface 等问题；rounded corner 也不在 surface 原语里。
- 这是 macOS 上最优的零拷贝路线，但需要维护 GPUI fork，不能作为跨平台方案。

### 方案 C：GPUI Fast 的 ExternalGpu composition

- `gpui-fast 0.1.12` 提供 `WindowComposition::create_external_gpu_surface` 和 `CompositionSurfaceKind::ExternalGpu`；macOS 适配层使用 layer-backed `NSView`，可以承载 `CAMetalLayer`。
- 需要把整个应用从 `gpui-pre` 切到 `gpui-fast`，包括 `gpui-kit/gpui-fast` 和直接依赖的 `gpui`；这不是单个预览组件的局部改动。
- 当前核对到的 composition 实现以 macOS 为主，不能据此承诺 Windows/Linux 同等能力。
- 不走 composition 的原生 child view 会盖住 GPUI 弹层，且在 gpui-pre 下无法被父容器正确裁剪，不适合皮肤页这种有弹层和圆角的区域。

### 方案 D：继续用 2D 绘制近似

不可行。`paint_path` 只接受纯色，`paint_quad` 不能做透视纹理映射；无法用 GPUI 现有 2D 原语正确绘制带 UV 的 3D 长方体。

## 建议

1. 先加测量，不先换渲染器：分别记录光栅、转换、上传和帧间隔；确认高 CPU 的实际归属。
2. 修帧调度：用常驻渲染 worker + “最新姿态”邮箱替代每帧 spawn；交互时 60 fps，停止拖动并稳定后再做 2× 超采样；页面隐藏或窗口失焦时暂停；避免每帧无谓地重建纹理。
3. 如果测量确认光栅仍是瓶颈，或要加阴影、披风、更高分辨率，再做方案 A；以 1.75 ms/帧为基线，GPU 回读方案必须证明自己更快，而不是只证明“用了 GPU”。
4. 只有明确要求零拷贝且接受 macOS 单平台和 GPUI fork 时，才评估方案 B；只有愿意整体迁移渲染后端时，才评估方案 C。

## 参考

- MineSkin 文档：<https://docs.mineskin.org/docs/mineskin-api/get-a-list-of-skins>
- MineSkin 最佳实践：<https://docs.mineskin.org/docs/guides/api-best-practises>
- Blessing Skin 源码：<https://github.com/bs-community/blessing-skin-server>
- LittleSkin 皮肤库：<https://littleskin.cn/skinlib>
- Crafty Public API：<https://crafty.gg/public-api>
- Ely.by 皮肤系统：<https://docs.ely.by/en/skins-system.html>
- GPUI `Surface`：`gpui-pre-0.3.10/src/elements/surface.rs`
- GPUI Metal surface 断言：`gpui-pre-apple-0.3.10/src/metal_renderer.rs`
- GPUI Fast composition：`gpui-fast-0.1.12/src/fast/composition.rs`
- 现有皮肤页记录：[skin_page.md](skin_page.md)
