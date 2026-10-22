# 本地构建 / Build from source

[简体中文介绍](../README.md) · [English introduction](../README.en.md)

## Download a preview

The [v0.1.0 developer preview](https://github.com/mymyme/cilai/releases/tag/v0.1.0) provides `cilai-personal-workbench-0.1.0-macos-arm64.zip` for **macOS 13.5 or later on Apple Silicon**. It bundles the official Codex CLI / App Server **0.163.0** runtime; no separate Codex installation or reuse of an existing `~/.codex` directory is needed. Windows and macOS x64 packages are not included in this release. macOS 13.5 is the minimum deployment target, not evidence of acceptance on an actual 13.5 system; that validation remains pending.

This ZIP is ad-hoc signed, without a Developer ID signature or Apple notarization. macOS may block its first launch. Automatic updates remain disabled. See the [release notes](releases/v0.1.0.md) and [compatibility record](COMPATIBILITY.md) before use.

To verify the download, save the ZIP and `SHA256SUMS` from the same release into one directory and run:

```sh
shasum -a 256 -c SHA256SUMS
```

The remaining instructions describe building from source.

## Checkout layout

Keep this repository at `06代码索引/cilai-workbench` under a dedicated parent directory. The current build derives its paths from that layout; it does not support placing a checkout at an arbitrary flat path.

```sh
git clone https://github.com/mymyme/cilai.git cilai-personal/06代码索引/cilai-workbench
cd cilai-personal/06代码索引/cilai-workbench
```

The commands work in a macOS terminal and Windows PowerShell. Git creates intermediate directories for the clone destination. All standard build staging, package dependencies, downloads, and outputs go to `cilai-personal/.personal-build`. Source-run data goes to `.personal-run/cn.cilai.personal.workbench`. These directories are local state and are not part of the repository.

## Prerequisites

| Dependency | Required version / environment |
| --- | --- |
| Node.js + npm | 24.18.0; use this Node on PATH |
| Rust | 1.97.1 with clippy, rustfmt and rust-docs (official runtime copyright and license materials) |
| macOS | Native arm64 or x64 host, Xcode Command Line Tools; Swift, PDFKit, AppKit |
| Windows | Windows 10 22H2 x64 or a later client x64 release, PowerShell 5.1+, MSVC C++ Build Tools, Windows SDK, CMake, Rust MSVC x64 target |
| cargo-deny | 0.20.2 for the full quality gate |

On Windows, `CILAI_PERSONAL_MSVC_ROOT` can point to an already-installed Microsoft toolchain. The build does not install system components or modify machine-wide settings.

## Standard local package

```sh
node --version
npm run check:toolchain
node scripts/personal-build.mjs
npm run check:conversation-package
npm run package
```

The standard entry stages current source, installs locked npm dependencies in the build root, builds the interface and first-party Rust services, prepares the pinned official runtime, and packages a testing build. It also checks a new isolated profile opening without a model connection. No product account, existing Codex login, model key, enterprise checkout, or maintainer signing certificate is required to build.

A network connection is needed for dependencies and official runtime downloads that are not yet cached. Checksums and version checks are mandatory; missing or changed artifacts cause an explicit failure. Preserve the bundled runtime manifests and notices. First-party native packages omit maintainer-specific build paths and debug-file paths; verify the packaged binaries before distribution.

The included shared UI source and tarball are checked for consistency. A normal build does not regenerate the tarball or rewrite dependency locks. To intentionally change that component, review its source and notices, run `npm run sync:conversation-package`, and review the resulting archive and lock changes before building.

## Output and launch

Output paths are relative to the dedicated parent directory:

| Platform | Testing package |
| --- | --- |
| macOS arm64 | `.personal-build/target/desktop-electron-testing/mac-arm64/cilai-personal-workbench.app` |
| macOS x64 | `.personal-build/target/desktop-electron-testing/mac/cilai-personal-workbench.app` |
| Windows x64 | `.personal-build/target/desktop-electron-testing/win-unpacked/cilai-personal-workbench.exe` |

`npm run launch` performs the same build and opens the personal package. Quit a running personal testing package normally before rebuilding. The build checks its own package path; do not kill applications by a shared process name.

macOS local packages use ad-hoc signing. The current distribution is `local`, with an empty update source and no automatic upload. The v0.1.0 GitHub preview is a ZIP archive of this local package, distributed separately from the updater. Formal signing, portable, and online-update entry points remain unavailable until their own distribution requirements are completed. A local build or public preview ZIP is not a Developer ID-signed or notarized release.

## Checks

The full quality entry also stages and manages dependencies in the dedicated build root:

```sh
# macOS
./script/quality.sh
```

```powershell
# Windows
powershell.exe -NoProfile -ExecutionPolicy Bypass -File script/quality.ps1
```

It includes dependency audits, types, tests, UI and sidecar builds, Rust checks, protocol checks, and cargo-deny. Each check reports its own result. See [compatibility](COMPATIBILITY.md) for the validation boundary; this document does not assert that the full quality entry has passed on a clean machine.

## Runtime data

The application ID is `cn.cilai.personal.workbench`. Packaged application data uses this namespace under the platform's application-data root. Browser state, credentials, native Codex state, and business data are isolated from the enterprise edition and existing user Codex directories. Configure model authorization in the personal workbench UI after startup.

Keep API keys, authentication files, databases, logs, build artifacts, screenshots containing private data, and local agent settings out of commits.
