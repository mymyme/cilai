# Supplemental upstream notices

These files supplement notices that are absent from published dependency archives. Their exact source URLs, revisions, byte lengths and SHA-256 digests are pinned in `provenance.json`; builds verify the stored bytes and do not retrieve notices over the network.

- `ts-rs` and `ts-rs-macros` use their release commit's original MIT license, including the upstream copyright notice.
- The `objc2` family uses the original repository notice from each crate's `.cargo_vcs_info.json` commit. That notice includes the upstream discussion of Apple SDK-derived bindings; it remains unchanged.
- `lazy-val` 1.0.5 is declared MIT by the original package metadata at the npm release's `gitHead`. Neither the npm archive nor that upstream repository revision contains a standalone license text. The original package declaration and author are retained.
- `MIT-standard-template.txt` is the unmodified SPDX MIT template from license-list-data v3.27.0. It is supplied alongside the `objc2` and `lazy-val` declarations because those sources refer to MIT without including its full text. Template placeholders are not substituted with invented years or copyright claims.

The application package also carries the original license and notice files found in its locked production npm dependency closure and the selected native Cargo target graph. This collection includes build-time and installed optional dependencies conservatively; it does not claim that every listed package is embedded in the final executable. See `Contents/Resources/licenses/dependencies/manifest.json` in a macOS package for the resolved list and digests.
