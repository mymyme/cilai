#!/usr/bin/env bash
set -euo pipefail

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
REQUIRED_NODE_VERSION="v24.18.0"
NVM_SCRIPT="/Users/$BUILD_USER/.nvm/nvm.sh"

if [[ "$(node --version 2>/dev/null || true)" != "$REQUIRED_NODE_VERSION" ]]; then
  if [[ ! -s "$NVM_SCRIPT" ]]; then
    echo "质量检查需要 Node ${REQUIRED_NODE_VERSION#v}，且未找到 NVM：$NVM_SCRIPT" >&2
    exit 1
  fi
  unset npm_config_prefix
  source "$NVM_SCRIPT"
  nvm use "${REQUIRED_NODE_VERSION#v}" >/dev/null
fi

if [[ "$(node --version)" != "$REQUIRED_NODE_VERSION" ]]; then
  echo "质量检查需要 Node ${REQUIRED_NODE_VERSION#v}，当前为 $(node --version)" >&2
  exit 1
fi

if [[ -z "${CILAI_BUILD_LOCK_TOKEN:-}" ]]; then
  node "$PROJECT_ROOT/scripts/build-lock.mjs" -- bash "$PROJECT_ROOT/script/quality.sh" "$@"
  exit $?
fi
node "$PROJECT_ROOT/scripts/build-lock.mjs" --check

cd "$PROJECT_ROOT"

# iCloud 源码目录不直接承载 Cargo 构建；质量检查与打包共用同一暂存清单。
if [[ "${CILAI_STAGED_BUILD:-0}" != "1" && "$PROJECT_ROOT" != "$BUILD_ROOT" ]]; then
  echo "[stage] 统一在 $BUILD_ROOT 校验"
  node "$PROJECT_ROOT/scripts/stage-build.mjs"
  (cd "$BUILD_ROOT" && npm ci --silent --no-audit)
  (cd "$BUILD_ROOT" && npm --prefix apps/desktop ci --silent --no-audit)
  (cd "$BUILD_ROOT" && node apps/desktop/scripts/prepare-electron-runtime.mjs)
  CILAI_STAGED_BUILD=1 "$BUILD_ROOT/script/quality.sh"
  exit 0
fi

npm run check:toolchain
npm audit --audit-level=high
npm run typecheck
npm test
npm run build
./script/prepare_sidecar.sh debug
node apps/desktop/scripts/stage-binaries.mjs --profile debug
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --quiet -p contexthub-protocol --bin generate-contracts -- --check

if [[ "$(cargo deny --version)" != "cargo-deny 0.20.2" ]]; then
  echo "cargo-deny 版本不匹配：要求 0.20.2" >&2
  exit 1
fi

cargo deny check advisories licenses bans sources

