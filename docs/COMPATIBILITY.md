# 兼容状态 / Compatibility

Recorded: **2026-10-22**. This is a developer preview. Source implementation, simulated tests, native builds, real service calls, and formal releases are separate validation levels.

## Platforms

| Platform | Current evidence | Remaining work |
| --- | --- | --- |
| macOS arm64 | Local Electron 44.4.4 package built; isolated startup without a model; model-setting and receipt recovery after restart | Broader first-release business acceptance, native notification delivery/click, clean-machine build, formal signing |
| macOS x64 | Source and official runtime target present | Native package and business validation |
| Windows 10 22H2 x64 and later client x64, including Windows 11 | Shared source and selected platform contracts exist | Native build, launch, security and complete business validation |
| Linux | No desktop packaging target registered | Outside the current desktop platform scope |

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

The latest local package changed documentation and licensing metadata after the real-model batch; restart restored its saved receipts without making another model call. Documentation changes are not a new provider acceptance run.

## Known gaps

- Arbitrary local MCP command isolation on Windows, full child-process lifecycle support, and the local-command broker are incomplete. Unsupported execution is explicitly rejected.
- If a browser HTTP-cache cleanup acknowledgement is missing, proof that the full Chromium process tree exited is incomplete. The maintenance fence remains held; cleanup is not repeated automatically.
- Native notification delivery and click handling still need system-level evidence.
- Backup consistency, restore failure protection, permission revocation, connection changes, automation shutdown, memory shutdown, and app-upgrade failure protection have component or selected integration evidence; final combinations and platforms still require acceptance.
- Full third-party redistribution review, both-platform clean-machine builds, and formal signed distribution remain incomplete.

## Dependency audit snapshot

cargo-deny **0.20.2** completed current-graph license, source, duplicate-version, and RustSec checks with exit code 0. Unused-license allowances and duplicate-version warnings remain recorded.

Both npm locks passed the configured high-severity audit threshold. This is not a zero-vulnerability result: the workspace reported **2 low and 2 moderate** records; the desktop build dependency tree reported **8 moderate** records. These are a dated snapshot, not a guarantee about later advisories. Dependency versions and audit rules were not changed to suppress findings.

## Source submission and releases

Publishing this repository does not publish an application installer or enable online updates. The current testing package is local and ad-hoc signed on macOS. There is no formally signed or notarized release in this submission. The enterprise website is [www.cilai.com.cn](https://www.cilai.com.cn).
