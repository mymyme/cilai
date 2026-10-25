#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-debug}"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_USER="$(id -un)"
BUILD_ROOT="$(node "$PROJECT_ROOT/scripts/personal-build.mjs")"
export CILAI_BUILD_ROOT="$BUILD_ROOT"
export CARGO_TARGET_DIR="$BUILD_ROOT/target"
export CARGO_HOME="$BUILD_ROOT/toolchains/cargo"
export npm_config_cache="$BUILD_ROOT/cache/npm"
export ELECTRON_CACHE="$BUILD_ROOT/cache/electron"
export electron_config_cache="$BUILD_ROOT/cache/electron/Cache"
export ELECTRON_BUILDER_CACHE="$BUILD_ROOT/cache/electron-builder"
mkdir -p "$BUILD_ROOT/config"
touch "$BUILD_ROOT/config/npmrc" "$BUILD_ROOT/config/npmrc-global"
export npm_config_userconfig="$BUILD_ROOT/config/npmrc"
export npm_config_globalconfig="$BUILD_ROOT/config/npmrc-global"
TARGET_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
export MACOSX_DEPLOYMENT_TARGET=13.5

if [[ "$PROJECT_ROOT" != "$BUILD_ROOT" ]]; then
  echo "sidecar 只能在统一构建目录中通过 npm run package 调用" >&2
  exit 1
fi

# 平台正向列举：脚本本身仅在 macOS 与 Linux 上以 bash 运行（Windows 原生入口见
# script/prepare_sidecar.ps1），因此这里只需区分「运行本脚本的宿主属于哪个 macOS
# 架构」，不承载 Windows 分支——按词来工作台 Windows 端适配技术方案第 2 节，条件分支
# 一律正向列举，缺失分支应当报错而不是静默落入否定分支。
case "$TARGET_TRIPLE" in
  aarch64-apple-darwin) SWIFT_TARGET="arm64-apple-macosx$MACOSX_DEPLOYMENT_TARGET" ;;
  x86_64-apple-darwin) SWIFT_TARGET="x86_64-apple-macosx$MACOSX_DEPLOYMENT_TARGET" ;;
  *)
    echo "本脚本（bash）仅支持 macOS 宿主（aarch64-apple-darwin / x86_64-apple-darwin），当前为 $TARGET_TRIPLE；Windows 原生构建请使用 script/prepare_sidecar.ps1" >&2
    exit 1
    ;;
esac

case "$PROFILE" in
  debug)
    PROFILE_DIR="debug"
    ;;
  release)
    PROFILE_DIR="release"
    ;;
  *)
    echo "usage: $0 [debug|release]" >&2
    exit 2
    ;;
esac

cd "$PROJECT_ROOT"
if [[ -z "${CARGO_ENCODED_RUSTFLAGS:-}" && -n "${RUSTFLAGS:-}" ]]; then
  read -r -a USER_RUST_FLAGS <<< "$RUSTFLAGS"
  printf -v CARGO_ENCODED_RUSTFLAGS '%s\x1f' "${USER_RUST_FLAGS[@]}"
  CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS%$'\x1f'}"
fi
RUST_SYSROOT="$(rustc --print sysroot)"
for mapping in "$BUILD_ROOT=/cilai/build" "$CARGO_HOME=/cilai/cargo" "$RUST_SYSROOT=/cilai/toolchain"; do
  CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+${CARGO_ENCODED_RUSTFLAGS}$'\x1f'}--remap-path-prefix=$mapping"
done
export CARGO_ENCODED_RUSTFLAGS
export CARGO_PROFILE_DEV_STRIP=debuginfo
export CARGO_PROFILE_RELEASE_STRIP=debuginfo
export CC_SHELL_ESCAPED_FLAGS=1
NATIVE_PATH_MAP="$(node -e 'console.log(JSON.stringify("-ffile-prefix-map=" + process.argv[1] + "=/cilai/build"))' "$BUILD_ROOT")"
export CFLAGS="${CFLAGS:+$CFLAGS }$NATIVE_PATH_MAP"
export CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$NATIVE_PATH_MAP"
# 宿主服务与应用服务一并构建。此前它不在这里：暂存只做取件不做编译，于是改了宿主服务
# 也进不了安装包，除非恰好手工构建过——表现为新命令在包里报"命令未迁移到宿主服务"。
if [[ "$PROFILE" == "debug" ]]; then
  cargo build -p contexthub-app-server -p cilai-host-service
else
  cargo build -p contexthub-app-server -p cilai-host-service --release
fi
# 按构建配置分目录：两种配置此前写同一个路径，后跑的覆盖先跑的。以正式配置暂存时
# 取到的可能是调试配置留下的产物，而两者文件名一致，谁也看不出来。
SIDECAR_DIR="target/sidecar/$PROFILE"
mkdir -p "$SIDECAR_DIR"
node scripts/prepare-personal-node.mjs "$SIDECAR_DIR" "cilai-node-$TARGET_TRIPLE"
cp "target/$PROFILE_DIR/contexthub-app-server" "$SIDECAR_DIR/cilai-app-server-$TARGET_TRIPLE"
test -x "$SIDECAR_DIR/cilai-app-server-$TARGET_TRIPLE"

# codex-runtime 保留官方完整发行包布局，源码及 Cargo.lock 不作任何改写。
node scripts/prepare-codex-runtime.mjs "$SIDECAR_DIR"
test -x "$SIDECAR_DIR/codex-runtime/bin/codex-app-server"

PDF_RENDERER="$SIDECAR_DIR/cilai-pdf-renderer-$TARGET_TRIPLE"
SWIFT_FLAGS=(-target "$SWIFT_TARGET" -framework PDFKit -framework AppKit -debug-prefix-map "$BUILD_ROOT=/cilai/build" -file-prefix-map "$BUILD_ROOT=/cilai/build")
if [[ "$PROFILE" == "release" ]]; then
  SWIFT_FLAGS=(-O "${SWIFT_FLAGS[@]}")
else
  SWIFT_FLAGS=(-Onone "${SWIFT_FLAGS[@]}")
fi
SWIFT_CACHE="$BUILD_ROOT/tmp/swift-module-cache"
mkdir -p "$SWIFT_CACHE"
CLANG_MODULE_CACHE_PATH="$SWIFT_CACHE" SWIFT_MODULE_CACHE_PATH="$SWIFT_CACHE" \
  node scripts/run-swift.mjs "${SWIFT_FLAGS[@]}" script/pdf_renderer.swift -o "$PDF_RENDERER"
test -x "$PDF_RENDERER"
node scripts/verify-native-build-paths.mjs "$BUILD_ROOT" "$RUST_SYSROOT" \
  "$SIDECAR_DIR/cilai-app-server-$TARGET_TRIPLE" "target/$PROFILE_DIR/cilai-host-service" "$PDF_RENDERER"
