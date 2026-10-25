# 兼容状态 / Compatibility

Updated: **2026-10-25**. Search and document-preview boundaries were checked against source on **2026-10-25**. Model-validation and Rust-audit evidence below was recorded on **2026-10-22**. Office component tests, isolated Chrome rendering and the workspace npm audit were refreshed on **2026-10-25**; these do not establish new Electron-package, service or platform acceptance. This is a developer preview. Source implementation, simulated tests, native builds, real service calls, and formal releases are separate validation levels.

## Platforms

| Platform | Current evidence | Remaining work |
| --- | --- | --- |
| macOS 13.5+ arm64 | v0.1.1 preview ZIP; local Electron 44.9.1 package built; isolated startup without a model and seven navigation entries checked | Broader first-release business acceptance, native notification delivery/click, actual macOS 13.5 hardware, clean-machine build, formal signing |
| macOS x64 | Source and official runtime target present | Native package and business validation |
| Windows 10 22H2 x64 and later client x64, including Windows 11 | Shared source and selected platform contracts exist | Native build, launch, security and complete business validation |
| Linux | No desktop packaging target registered | Outside the current desktop platform scope |

macOS 13.5 is the package's minimum deployment target. Acceptance on an actual macOS 13.5 system remains pending.

Windows and macOS results are recorded independently. A passing macOS build or static Windows contract is not Windows native acceptance.

## Runtime and model connections

The runtime is the official **Codex App Server 0.163.0** distribution. The retained **0.153.4** source snapshot is historical and is not the executable used by the application.

| Connection | Current evidence |
| --- | --- |
| Custom Responses: Agnes, `agnes-3.0-flash` | One complete eight-layer GUI validation passed: configuration, network, authentication, Responses protocol, catalog, inference, streaming, and tools |
| OpenAI API key | Connection and vault implementation present; real authorized OpenAI validation pending |
| Official ChatGPT authorization | Isolated runtime-managed device-code lifecycle implemented; real authorized login, expiry, renewal, cancellation and logout pending |
| Ollama / LM Studio and other local models | Connection interfaces and capability records implemented; actual service/version/model combinations pending |
| Other Responses providers/models | Not established by the Agnes result; validate each actual combination |

Earlier Agnes attempts produced HTTP 400 for empty-tool selection fields and separate empty-reply failures. The workbench normalizes the empty-tool fields before the initial request, preserves explicit/nonempty tool choices, and reports empty replies as failures. A later successful batch does not erase earlier failures or establish long-term provider reliability. No failed user task was automatically replayed.

The model evidence above was recorded before the v0.1.0 release packaging. A later local package restored saved receipts after restart without another model call. Documentation, version, and packaging changes are not a new provider acceptance run.

## Known gaps

- Arbitrary local MCP command isolation on Windows, full child-process lifecycle support, and the local-command broker are incomplete. Unsupported execution is explicitly rejected; see the [isolation requirements](MCP_SANDBOX.md).
- Local file search includes names, paths and complete UTF-8 text up to 2 MiB (including BOM). Hidden paths, binary/Office/PDF documents, other encodings and larger text files are outside body indexing; supported file names remain searchable. Text is refreshed by the normal workspace indexer. Conversation and business-object search use their separate existing providers.
- PowerPoint preview reads stored chart data for basic bar, line, area, pie, doughnut, scatter and radar charts, with category labels and supported axis/grouping settings. It reads SmartArt shapes and text from the package's saved diagram drawing. Missing chart data, missing computed SmartArt layout, 3D/unknown chart types, secondary value axes and unsupported combinations show an explicit object error. Negative pie/doughnut values, negative percentage stacks and radar charts with missing or negative values are explicitly unsupported. It does not reconstruct arbitrary SmartArt layout algorithms or refresh external chart workbooks.
- Spreadsheet preview recalculates supported scalar formulas and dependency chains in a worker, including cross-sheet references, ranges, supported named references and translated shared formulas. Original cached values remain available for comparison and never substitute for failed calculations. Unknown functions, cycles, unsupported array/data-table formulas, unresolved names, external workbooks and resource limits produce visible calculation errors. Arithmetic and TEXT formatting honor the workbook's recorded date system; date functions in 1904-system workbooks remain explicitly unsupported. This is a preview calculator, not complete Excel compatibility or an editor.
- On 2026-10-25, an isolated system-Chrome page exercised real workers, PPTX charts/SmartArt, spreadsheet recalculation and explicit formula errors, plus Mermaid 12 flowcharts and KaTeX math. It used synthetic local packages without user data. This establishes browser component behavior only; real customer Office files, current Electron packaging, Windows and macOS Intel still need separate acceptance. These previews do not promise full Office rendering fidelity.
- If a browser HTTP-cache cleanup acknowledgement is missing, proof that the full Chromium process tree exited is incomplete. The maintenance fence remains held; cleanup is not repeated automatically.
- Native notification delivery and click handling still need system-level evidence.
- Backup consistency, restore failure protection, permission revocation, connection changes, automation shutdown, memory shutdown, and app-upgrade failure protection have component or selected integration evidence; final combinations and platforms still require acceptance.
- Full third-party redistribution review, both-platform clean-machine builds, and formal signed distribution remain incomplete.

## Dependency audit snapshot

cargo-deny **0.20.2** completed current-graph license, source, duplicate-version, and RustSec checks with exit code 0. Unused-license allowances and duplicate-version warnings remain recorded.

Workspace npm lock: the **2026-10-25** audit returned **0 vulnerabilities** after updating Mermaid to 12.2.0, DOMPurify to 3.4.17, KaTeX to 0.18.10 and Vitest to 4.1.12. The formula calculator is pinned to fast-formula-parser 1.0.19. Existing Mermaid rendering uses the supported dagre/classic configuration, and real Chrome rendering was checked after the update. The production notice collector verified original notices for all 248 installed frontend production components, including the new calculator's dependency closure. No advisory, dependency or audit rule was suppressed.

Desktop npm lock: the **2026-10-22** audit recorded **8 moderate** findings and passed the configured high-severity threshold. The **2026-10-25** audit returned **0 vulnerabilities** after pinning Electron to 44.9.1 and refreshing the existing compatible dependency ranges to Undici 7.30.1 / 6.29.1 and fast-uri 3.1.9. Electron remains on major 44; no dependency override or advisory suppression was added. The Electron update includes the [official compromised-certificate mitigation](https://github.com/electron/electron/security/advisories/GHSA-jr5w-w6x2-84p9). Audit results are dated snapshots, not guarantees about future advisories.

On **2026-10-25**, the installed source and personal-stage Electron distributions both matched 44.9.1. Existing macOS hidden-window fixtures passed with real Electron: browser guest preload/Node isolation and screenshot, personal-frame storage isolation and confirmed detachment, and page-AI SDK execution/reload revocation/self-approval rejection. The three fixture files contained 22 passing checks; service replies in the page-AI fixture are contract data, without a real Host, model inference or complete application session. These checks do not establish Windows native acceptance. This source dependency refresh does not update or replace the previously published **v0.1.0** preview package, whose historical Electron 44.4.4 build evidence remains separate.

## Source submission and releases

The [v0.1.1 developer preview](https://github.com/mymyme/cilai/releases/tag/v0.1.1) provides a ZIP for **macOS 13.5 or later on Apple Silicon (arm64)**, with a `SHA256SUMS` file. No Windows or macOS Intel package is included. See the [bilingual release notes](releases/v0.1.1.md). The previous v0.1.0 archive remains available.

For v0.1.1 on **2026-10-26**, the standard personal build produced the macOS arm64 application with Electron 44.9.1, bundled notices, and an isolated no-model startup check. The release preparation passed 1338 frontend tests, 522 Node/shell/build checks with 12 conditional skips, type checking, workspace/all-target Clippy and formatting, protocol consistency, and the system-proxy route-isolation test. Both npm dependency trees reported zero audit findings. These are local source/package checks; the provider evidence above remains historical and was not repeated for this release.

The package includes the pinned official Codex CLI / App Server **0.163.0** runtime and uses dedicated workbench configuration and state. It does not reuse an existing user's `~/.codex` profile. It is ad-hoc signed, without a Developer ID signature or Apple notarization; macOS may block the first launch. Online updates remain disabled.

A public preview download does not mark the complete first-release scope as accepted or establish support for untested platforms and model combinations. The enterprise website is [www.cilai.com.cn](https://www.cilai.com.cn).
