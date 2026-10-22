# 第三方来源 / Third-party sources

First-party source and resources are licensed under [Apache-2.0](../LICENSE), with attribution in [NOTICE](../NOTICE). Third-party licenses and notices remain applicable to their components.

| Component | Pinned source / version | License and provenance |
| --- | --- | --- |
| Official Codex App Server | 0.163.0; platform artifacts and checksums in the runtime manifest | Apache-2.0; same-version official LICENSE, NOTICE and source provenance are preserved under `third_party/openai-codex-notices/` and verified during runtime preparation |
| Historical Codex source | 0.153.4 | Preserved under `third_party/openai-codex/` with its upstream notices; see the source-snapshot record. It is separate from the actual runtime |
| Node.js | 24.18.0 | The managed binary is accompanied by `sidecar/node-LICENSE.txt`, including Node's own third-party notices |
| Electron / Chromium | Electron 44.4.4 | Original Electron LICENSE and Chromium notices are copied from the matching official distribution into `licenses/electron/` |
| PDF.js | pdfjs-dist 6.3.289 | Original top-level and used no-WASM decoder licenses are preserved in `licenses/pdfjs-dist/`; assets and matching versions are checked |
| Shared UI | `@cilai/app-ui` 1.1.0 | First-party source in `packages/app-ui/` and a matching local tarball, both with LICENSE/NOTICE |
| Windows Git runtime | MinGit 2.56.0.windows.2 | Preparation records the archive and source URLs and requires upstream notices. Full Windows distribution/source-offer review remains pending |
| Other Rust and npm dependencies | Cargo.lock and the two npm lockfiles | Individual component licenses apply; locks are reproducibility inputs rather than a complete redistribution notice bundle |

Source references:

- [Official runtime manifest](../third_party/openai-codex.release.json)
- [Runtime preparation notes](../third_party/openai-codex.runtime.md)
- [Retained source-snapshot record](../third_party/openai-codex.source.md)
- [Shared UI archive consistency manifest](../packages/conversation-package.json)
- [Cargo lockfile](../Cargo.lock), [workspace npm lockfile](../package-lock.json), [desktop npm lockfile](../apps/desktop/package-lock.json)

The build checks the pinned official Codex notices, first-party notices, and selected packaged runtime notices. This source submission does not claim that every future binary redistribution condition has been completed. Before distributing installers, finish the full dependency/asset notice inventory, any corresponding-source obligations, and native platform validation described in [compatibility](COMPATIBILITY.md).

Model service authorization, account access, usage charges, and service terms are separate from the source license.

The source submission excludes generated tool caches and editor-local settings from the retained historical tree. Existing source files, license texts, attribution, and historical modification records are retained; the snapshot remains separate from the official runtime.
