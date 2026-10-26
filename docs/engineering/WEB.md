# Web 站点与 GitHub Pages

状态：已启用

日期：2026-10-27

## 边界

`web/` 是 RMCL 的介绍页，使用 Bun、TypeScript 和 Three.js 构建。它不参与桌面启动器编译，也不读取用户账号、实例或启动器配置。桌面应用仍是唯一的产品入口。

站点部署到 GitHub Pages：

- 地址：<https://driftcell.github.io/RMCL/>
- 仓库 Pages 构建源：GitHub Actions（`build_type = workflow`）
- 不使用自定义域名，也不依赖 Pages 服务端构建。

## 本地命令

在 `web/` 目录运行：

```bash
bun install --frozen-lockfile
bun run dev      # 本地热更新服务
bun run check    # tsc --noEmit
bun run build    # 输出 web/dist/
bun run serve    # 直接预览 index.html
```

`web/dist/` 和 `web/node_modules/` 已被 `web/.gitignore` 忽略，不提交构建产物。生产构建使用相对资源路径，因此部署在 `/RMCL/` 项目子路径下也能正常加载。

## CI 检查

`.github/workflows/ci.yml` 的 `web` job 在以下路径变化时运行：

- `web/**`
- `.github/workflows/ci.yml`
- `.github/workflows/pages.yml`

job 固定使用 Bun 1.4.4，执行 `bun install --frozen-lockfile`、`bun run check` 和 `bun run build`。`CI success` gate 允许 Web job 在纯 Rust/文档改动时跳过，但 Web job 失败或取消会使 gate 失败。

## Pages 部署

`.github/workflows/pages.yml` 在以下情况运行：

- 推送到 `main` 且改动 `web/**` 或 `.github/workflows/pages.yml`
- 手动 `workflow_dispatch`

流程：

1. 检出仓库并安装 Bun 1.4.4。
2. 在 `web/` 执行冻结锁文件安装、类型检查和构建。
3. 通过 `actions/configure-pages`、`actions/upload-pages-artifact` 上传 `web/dist/`。
4. 由 `actions/deploy-pages` 发布到 `github-pages` environment。

workflow 使用 `pages: write` 和 `id-token: write` 权限，不写入仓库内容。部署并发组固定为 `pages`，不在部署完成前取消旧部署。

## 验证

- 本地运行 `bun run check` 和 `bun run build`，确认 `web/dist/index.html` 使用相对资源路径。
- 合并到 `main` 后检查 Pages workflow 的 build 和 deploy job 成功。
- 打开 <https://driftcell.github.io/RMCL/> 检查首页、截图、字体和 Three.js 场景；真实浏览器不报 404 或模块加载错误。
- PR 预览不部署到 Pages；PR 只运行 `web` CI job。

## 当前限制

- 没有 PR 预览环境、自定义域名、缓存头或访问分析。
- 字体仍从 Google Fonts 加载；离线或受限网络下会回退到系统字体。
- 页面版本号和桌面应用发布版本没有自动同步，更新发布时需要同步检查 `web/index.html`。
