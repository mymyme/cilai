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
| Other Rust and npm dependencies | Cargo.lock and the two npm lockfiles | The package includes original license, copyright and notice files with a resolved manifest under `licenses/dependencies/`; individual component licenses apply |
| Rust standard library and toolchain runtime support | Rust 1.97.1, commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452` | `licenses/rust-toolchain/` contains the original complete toolchain and standard-library copyright reports, all 12 installed license texts, and same-commit compiler-builtins/libm notices |

Source references:

- [Official runtime manifest](../third_party/openai-codex.release.json)
- [Runtime preparation notes](../third_party/openai-codex.runtime.md)
- [Retained source-snapshot record](../third_party/openai-codex.source.md)
- [Shared UI archive consistency manifest](../packages/conversation-package.json)
- [Cargo lockfile](../Cargo.lock), [workspace npm lockfile](../package-lock.json), [desktop npm lockfile](../apps/desktop/package-lock.json)

The build verifies the pinned official Codex notices, first-party notices, selected runtime notices, and the dependency notice bundle. The dependency bundle covers the installed production npm lock graph for the frontend and Electron main process, plus the target-specific Cargo dependency closure of the host and app servers. Build-time and installed optional components are included conservatively. Each component is recorded with its declared license, source, version and notice file hashes; Cargo notice text is compared byte for byte with the registry archive whose checksum is fixed in Cargo.lock. Missing notices, version drift or modified files stop packaging.

For the first macOS arm64 preview, this closure contains 238 frontend npm packages, 16 Electron-main npm packages and 193 registry crates. The final package manifest is the authoritative inventory. Ten optional npm packages for other targets are absent from this installed graph. This does not establish Windows or Intel Mac distribution readiness; native platform validation remains separately recorded in [compatibility](COMPATIBILITY.md).

Some registry packages omit their license files. The build includes original README license sections where provided, and fixed same-release upstream materials under [supplemental notices](../third_party/dependency-notices/README.md). `lazy-val` 1.0.5 declares MIT in its original package metadata, with its author, but publishes no standalone license text. Several `objc2` crates likewise refer to MIT without reproducing its text. Their original declarations are preserved alongside the unmodified SPDX MIT standard template; its placeholders are not presented as upstream copyright information. The `objc2` upstream notice's discussion of Apple SDK-derived bindings is also retained. This notice inventory is not a new legal determination of that upstream qualification.

The current macOS package contains no MinGit runtime. A future Windows package still requires the separate MinGit corresponding-source and distribution review, its native platform checks, and any additional obligations introduced by that target's resolved dependencies.

The Rust standard library and its runtime support are outside the application's Cargo metadata graph. Packaging separately checks the exact Rust release and source commit, then preserves `COPYRIGHT.html`, `COPYRIGHT-library.html` and the complete installed `share/doc/rust/licenses/` directory. These are the full official toolchain notices, including other-target and compiler-only components, rather than a claim that every listed component is linked into the app. The exact Rust source commit declares `compiler_builtins` 0.1.160 under `MIT AND Apache-2.0 WITH LLVM-exception AND (MIT OR Apache-2.0)`; its original combined license and the adjacent libm license are additionally pinned and shipped. The corresponding manifest records original relative paths and checksums without embedding the build machine's sysroot path. Missing documents, changed bytes, or a compiler version/commit mismatch stop packaging.

Model service authorization, account access, usage charges, and service terms are separate from the source license.

The source submission excludes generated tool caches and editor-local settings from the retained historical tree. Existing source files, license texts, attribution, and historical modification records are retained; the snapshot remains separate from the official runtime.
