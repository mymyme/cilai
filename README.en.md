<p align="center"><img src="assets/cilai-symbol-primary.svg" alt="Cilai" width="72"></p>

# Cilai Personal Workbench

**A local-first desktop AI workbench built on Codex CLI.**

Bring projects, files, persistent conversations, tools, and personal apps into one workspace, so AI can help carry work through to completion.

[简体中文](README.md) · [Build guide](docs/BUILDING.md) · [Compatibility](docs/COMPATIBILITY.md) · [Enterprise website](https://www.cilai.com.cn) · [Apache-2.0](LICENSE)

## Highlights

- **A local workspace without product registration.** Manage local projects, files, history, and settings before connecting a model.
- **Ongoing work organized around projects.** Keep conversations connected to workspaces, files, and artifacts, and return to them later.
- **Execution powered by Codex CLI.** Use the official App Server for persistent conversations, streamed events, tool execution, and approval handling, with adaptation in the workbench.
- **Independent model connections.** Connection interfaces cover OpenAI API keys, official ChatGPT authorization, custom Responses services, and local models. Model authorization is separate from local data ownership.
- **Composable tools and skills.** Manage skills and MCP connections alongside an embedded browser, terminal, and file tools, with user-controlled execution permissions.
- **Personal apps and page AI.** Run apps inside the main window, grant declared capabilities explicitly, and manage app versions and data separately.
- **Automation, review, and memory.** New profiles default to manual approval. Automatic review and background memory require explicit opt-in, with disclosure of services, history scope, and costs.
- **Data protection and visible operation.** Access backup and restore, storage management, diagnostics, and usage views. API keys are held in the workbench's local encrypted vault.

These capabilities are under active first-release development and acceptance testing. See [compatibility and known limits](docs/COMPATIBILITY.md) for platform and model-specific status.

## Codex CLI dependency

Cilai Personal Workbench depends on [OpenAI Codex CLI](https://github.com/openai/codex) for agent execution and integrates through the public [Codex App Server](https://developers.openai.com/codex/app-server/) protocol.

- The project currently pins the official **Codex App Server 0.163.0** distribution. Versions, download sources, and checksums are recorded in the [runtime manifest](third_party/openai-codex.release.json).
- The standard build prepares and verifies this distribution. It does not require reusing a system-wide Codex CLI installation or compiling the retained historical source snapshot.
- Codex runs with dedicated workbench configuration and state directories. The workbench does not read or copy an existing user's `~/.codex` configuration, authentication, history, or memory.
- This project provides the desktop interface, local data management, model connections, and permission adaptation while leaving official Codex source and algorithms unchanged.

Model calls require your own valid authorization. Catalog access, text inference, streaming, and tool compatibility are validated separately. Account access and charges are determined by the service provider.

## Build from source

This repository contains a **developer preview**. The standard build produces a local testing package. Install these prerequisites first:

| Tool | Version or requirement |
| --- | --- |
| Node.js | **24.18.0**, including npm |
| Rust | **1.97.1**, including clippy / rustfmt |
| macOS | Xcode Command Line Tools; Swift / PDFKit / AppKit |
| Windows | Windows 10 22H2 x64 or later client x64, PowerShell 5.1+, MSVC C++ Build Tools, Windows SDK, CMake |
| Full quality checks | cargo-deny **0.20.2** |

The build currently requires the checkout at `06代码索引/cilai-workbench`. Clone with this relative layout to keep source, builds, and runtime data separate:

```sh
git clone https://github.com/mymyme/cilai.git cilai-personal/06代码索引/cilai-workbench
cd cilai-personal/06代码索引/cilai-workbench
node --version
npm run check:toolchain
node scripts/personal-build.mjs
npm run check:conversation-package
npm run package
```

The npm commands work in a macOS terminal or Windows PowerShell. `npm run package` installs locked dependencies in the dedicated build root, builds the UI and local services, prepares the official runtime, and checks startup without a model connection.

```text
cilai-personal/
├── 06代码索引/cilai-workbench/   # This repository
├── .personal-build/             # Staging, dependencies, caches, and local packages
└── .personal-run/               # Source-run data
```

Use `npm run launch` to build and open the personal testing package. Quit a running personal testing package normally before rebuilding. See the [build guide](docs/BUILDING.md) for output paths, checks, and platform requirements.

## Current status

The project is a **developer preview** with first-release acceptance work still in progress. No formally signed distribution is available from this submission.

| Area | Current evidence |
| --- | --- |
| macOS arm64 | Local package build, startup without a model, and restart recovery verified |
| Custom Responses | One complete eight-layer validation passed for Agnes / `agnes-3.0-flash` |
| macOS Intel / Windows | Target platforms; native packages and business acceptance remain unverified |
| OpenAI API / ChatGPT authorization / local models | Connection implementations exist; real authorization and specific compatibility combinations remain to be validated |

The [compatibility record](docs/COMPATIBILITY.md) distinguishes implementation, simulated checks, package builds, and real calls. A passing combination does not establish support for every model or platform.

## Repository layout

```text
apps/desktop-ui/     React + TypeScript interface
apps/desktop/        Electron Shell and Rust Host
crates/             Domain, application, persistence, protocol, and runtime
packages/app-ui/    Bundled shared UI source
migrations/         Forward-only local database migrations
schemas/            Interface contracts
scripts/            Build, dependency preparation, and verification
third_party/        Official runtime manifests, notices, and historical source
```

See the [architecture](docs/ARCHITECTURE.md) and [third-party notes](docs/THIRD_PARTY.md).

## Enterprise edition

Visit **[www.cilai.com.cn](https://www.cilai.com.cn)** for the enterprise edition.

The personal open-source edition focuses on local workflows and operates independently of enterprise accounts, cloud spaces, and collaboration systems. “Personal” describes the feature scope; it does not restrict enterprise or commercial use. Third-party service authorizations and terms apply separately.

## License and contributions

First-party source and resources are licensed under the **[Apache License 2.0](LICENSE)**. See [NOTICE](NOTICE) for attribution. Third-party components, including Codex, Node, Electron, and PDF.js, retain their own licenses and required notices.

Report problems in [Issues](https://github.com/mymyme/cilai/issues) or contribute through a Pull Request. Include your platform, application and runtime versions, and reproduction steps, and remove keys, personal data, and sensitive logs.
