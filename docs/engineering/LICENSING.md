# 独立实现与许可

状态：已接受的工程路线

日期：2026-10-29

## 决策

RMCL 的原创代码以 MIT 许可路线准备。项目采用独立 Rust 架构和实现，HMCL 用于理解功能、兼容行为、文件格式及测试对照。

这项决策不代表 HMCL 的 GPLv3 许可被改变，也不表示其代码和资源可以随 RMCL 以 MIT 再许可。

## 工程前提

- 不直接复制或贴身翻译 HMCL 的 Java 源码、注释、测试样本、图标、翻译和其他素材。
- 通过公开协议、元数据格式、可观察行为和独立测试实现所需功能。
- 可以把 HMCL 作为行为对照实现，记录差异和兼容性问题；引用位置不等于复制实现。
- 引用 HMCL 源码时使用固定 commit，以便说明行为依据。
- 第三方 API、游戏元数据、Java 运行时和测试素材继续遵守各自条款。
- 发布前补齐 MIT `LICENSE`、Cargo 许可元数据、版权信息和第三方许可清单。

## authlib-injector

authlib-injector 使用 GNU Affero General Public License v3.0 或更高版本，并附带 “AUTHLIB-INJECTOR” 例外。该例外明确允许：

1. 在程序中打包未经修改的二进制形式，且不静态或动态链接该作品；
2. 通过其进程间通信接口（如 HTTP API）交互；
3. 将其作为 Java Agent 加载进 JVM。

RMCL 当前不随安装包分发 authlib-injector，而是在运行期从官方 API 或 BMCLAPI 镜像下载，并使用响应中的 SHA-256 校验后缓存。发布说明和第三方许可清单仍需注明项目来源、版本、许可证和下载地址。若未来改为打包，必须保留未修改二进制、完整许可证文本和来源说明。

项目主页：<https://github.com/yushijinhun/authlib-injector>

## 需要重新评估的情况

如果未来需要直接复用 HMCL 的代码、翻译、资源或大段测试数据，必须在合并或分发前重新评估衍生作品与 GPLv3 附加条款的影响。不能因为产品已改名为 RMCL，就直接继续采用 MIT。

本文记录的是项目工程约束，不替代正式法律意见。

调研背景见 [许可路线取舍](../../notes/bootstrap/licensing_decision.md)。
