<p align="center"><img src="assets/cilai-symbol-primary.svg" alt="词来" width="72"></p>

# 词来个人工作台

**基于 Codex CLI 的本地优先桌面 AI 工作台。**

把项目、文件、持续对话、工具与个人应用放在同一个工作空间，让 AI 从回答问题走向完成工作。

[English](README.en.md) · [构建指南](docs/BUILDING.md) · [兼容状态](docs/COMPATIBILITY.md) · [企业版官网](https://www.cilai.com.cn) · [Apache-2.0](LICENSE)

## 特点

- **打开即用的本地工作台**：无需注册产品账号。本地项目、文件、历史和配置不依赖模型连接即可管理。
- **围绕项目持续工作**：对话关联工作区和文件，保留过程与产物，支持在后续会话中继续推进。
- **Codex CLI 驱动执行**：通过官方 App Server 接入持续会话、流式事件、工具执行与审批交互，适配在工作台侧完成。
- **模型连接独立管理**：提供 OpenAI API Key、官方 ChatGPT 授权、自定义 Responses 和本地模型连接入口；模型授权与本地资料归属分离。
- **工具与技能可组合**：管理技能与 MCP 连接，结合内嵌浏览器、终端和文件能力处理实际任务，执行权限由用户控制。
- **个人应用与页面 AI**：应用内嵌主窗口，按声明和授权使用页面能力，应用数据与版本分别管理。
- **自动化、审核与记忆**：新资料默认人工确认；自动审核与后台记忆需显式启用，并说明服务、历史发送范围与费用。
- **资料保护与可观察运行**：提供备份恢复、存储管理、诊断及使用量入口；API 密钥保存在工作台本地加密库。

以上能力正在按首版范围开发与验收。具体平台、模型和已知限制见[兼容状态](docs/COMPATIBILITY.md)。

## 依赖 Codex CLI

词来个人工作台的智能体执行依赖 [OpenAI Codex CLI](https://github.com/openai/codex)，通过官方 [Codex App Server](https://developers.openai.com/codex/app-server/) 的公开协议集成。

- 当前锁定官方 **Codex App Server 0.163.0** 完整发行件；版本、下载来源和摘要见[运行时清单](third_party/openai-codex.release.json)。
- 普通构建会准备并校验这一发行件，不要求复用你在系统中安装的 Codex CLI，也不编译保留的旧版源码快照。
- Codex 使用个人版专属配置与状态目录，不读取或复制用户现有 `~/.codex` 的配置、认证、历史或记忆。
- 工作台负责界面、本地资料、连接与权限适配，保持官方 Codex 源码与算法不变。

模型接入仍需要用户自己的有效授权。连接目录可读、普通问答成功、流式可用和工具兼容分别验证；费用与账号权限由服务供应商决定。

## 从源码构建

当前提供开发预览源码，构建入口生成本地测试包。请先准备：

| 工具 | 版本或要求 |
| --- | --- |
| Node.js | **24.18.0**，包含 npm |
| Rust | **1.97.1**，含 clippy / rustfmt |
| macOS | Xcode Command Line Tools；Swift / PDFKit / AppKit |
| Windows | Windows 10 22H2 x64 或后续客户端 x64，PowerShell 5.1+、MSVC C++ Build Tools、Windows SDK、CMake |
| 完整质量检查 | cargo-deny **0.20.2** |

构建器目前要求源码位于 `06代码索引/cilai-workbench`。按下面的相对布局克隆，构建和运行目录会保持独立：

```sh
git clone https://github.com/mymyme/cilai.git cilai-personal/06代码索引/cilai-workbench
cd cilai-personal/06代码索引/cilai-workbench
node --version
npm run check:toolchain
node scripts/personal-build.mjs
npm run check:conversation-package
npm run package
```

上述 npm 命令可在 macOS 终端或 Windows PowerShell 执行。`npm run package` 会在独立构建根内按锁文件安装依赖、构建界面与本地服务、准备官方运行时，并执行个人版无模型启动检查。

```text
cilai-personal/
├── 06代码索引/cilai-workbench/   # 当前仓库
├── .personal-build/             # 暂存、依赖、缓存与本地测试包
└── .personal-run/               # 源码运行资料
```

启动使用 `npm run launch`。重建前正常退出正在运行的个人测试包。详细输出路径、检查入口与平台前提见[构建指南](docs/BUILDING.md)。

## 当前状态

项目处于 **开发预览**，首版仍在验收，尚无正式签名发行。

| 范围 | 当前记录 |
| --- | --- |
| macOS arm64 | 本地包构建、无模型启动及重启恢复已有验证 |
| 自定义 Responses | Agnes / `agnes-3.0-flash` 已完成一次八项完整验证 |
| macOS Intel / Windows | 目标平台；原生包与业务验收仍待完成 |
| OpenAI API / ChatGPT 授权 / 本地模型 | 已有接入实现，真实授权与具体兼容组合仍待验证 |

[兼容状态与已知缺口](docs/COMPATIBILITY.md)区分实现、模拟检查、构建和真实调用，不将一项通过推广到所有模型或平台。

## 项目结构

```text
apps/desktop-ui/     React + TypeScript 界面
apps/desktop/        Electron Shell 与 Rust Host
crates/             领域、应用、持久化、协议及运行时
packages/app-ui/    内置共享界面组件源码
migrations/         本地数据库前向迁移
schemas/            接口契约
scripts/            构建、依赖准备与验证
third_party/        官方运行时清单、通知与历史源码快照
```

设计边界见[架构说明](docs/ARCHITECTURE.md)，依赖与归属见[第三方说明](docs/THIRD_PARTY.md)。

## 企业版

需要了解企业版，请访问 **[词来官网 · www.cilai.com.cn](https://www.cilai.com.cn)**。

个人开源版聚焦本地个人工作流，独立于企业账号、云空间和企业协作系统。“个人版”描述产品功能范围，不限制企业或商业使用；第三方服务的授权与条款分别适用。

## 许可证与贡献

第一方源码及资源使用 **[Apache License 2.0](LICENSE)**，归属说明见 [NOTICE](NOTICE)。Codex、Node、Electron、PDF.js 等第三方组件保留各自许可证和必要声明。

欢迎通过 [Issues](https://github.com/mymyme/cilai/issues) 反馈问题，或提交 Pull Request。反馈时请注明平台、应用与运行时版本、复现步骤，并移除密钥、个人资料和敏感日志。
