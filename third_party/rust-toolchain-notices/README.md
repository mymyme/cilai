# Rust toolchain notices

The build reads the complete original `COPYRIGHT.html`, `COPYRIGHT-library.html` and all twelve files under `share/doc/rust/licenses/` from the installed, pinned Rust 1.97.1 toolchain. Their expected hashes are recorded here. The build checks both the compiler release and commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`; it does not copy the toolchain source into this repository.

The two small supplemental license files are unchanged originals from that exact Rust source commit: `library/compiler-builtins/LICENSE.txt` and `library/compiler-builtins/libm/LICENSE.txt`. The in-tree `compiler-builtins/compiler-builtins/Cargo.toml` identifies `compiler_builtins` 0.1.160 and declares `MIT AND Apache-2.0 WITH LLVM-exception AND (MIT OR Apache-2.0)`. This runtime support is outside the application's Cargo metadata graph.

The bundled `licenses/rust-toolchain/` is the complete official notice collection, including compiler-only and other-platform material. It is not a claim that every listed toolchain component is linked into the application. Original copyright attribution and template placeholders remain unchanged. `provenance.json` records the sources and hashes without a local machine path.
