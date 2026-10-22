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
NVM_SCRIPT="/Users/$BUILD_USER/.nvm/nvm.sh"
REQUIRED_NODE_VERSION="v24.18.0"
PROFILE="debug"
LAUNCH=0
LOCAL_TEST=1

for argument in "$@"; do
  case "$argument" in
    --release|--online-test) echo "个人版当前仅生成本地测试包，发布和在线更新测试尚未配置" >&2; exit 1 ;;
    --launch) LAUNCH=1 ;;
    *) echo "不支持的打包参数：$argument" >&2; exit 1 ;;
  esac
done
if [[ "$PROFILE" == "release" && "$LAUNCH" == "1" ]]; then
  echo "正式分发构建不通过开发启动入口运行" >&2
  exit 1
fi

if [[ "$(node --version 2>/dev/null || true)" != "$REQUIRED_NODE_VERSION" ]]; then
  if [[ ! -s "$NVM_SCRIPT" ]]; then
    echo "打包需要 Node ${REQUIRED_NODE_VERSION#v}，且未找到 NVM：$NVM_SCRIPT" >&2
    exit 1
  fi
  unset npm_config_prefix
  source "$NVM_SCRIPT"
  nvm use "${REQUIRED_NODE_VERSION#v}" >/dev/null
fi
if [[ "$(node --version)" != "$REQUIRED_NODE_VERSION" ]]; then
  echo "打包需要 Node ${REQUIRED_NODE_VERSION#v}，当前为 $(node --version)" >&2
  exit 1
fi

if [[ -z "${CILAI_BUILD_LOCK_TOKEN:-}" ]]; then
  node "$PROJECT_ROOT/scripts/build-lock.mjs" -- bash "$PROJECT_ROOT/script/package.sh" "$@"
  exit $?
fi
node "$PROJECT_ROOT/scripts/build-lock.mjs" --check

OUTPUT_CHANNEL="$(node "$PROJECT_ROOT/scripts/personal-build.mjs" --channel)"
OUTPUT_DIRECTORY="$(node "$PROJECT_ROOT/apps/desktop/scripts/package-output.mjs" "$OUTPUT_CHANNEL")"
APP="$(node "$PROJECT_ROOT/apps/desktop/scripts/package-output.mjs" "$OUTPUT_CHANNEL" --bundle)"
EXECUTABLE="$APP/Contents/MacOS/cilai-personal-workbench"
if [[ -d "$APP" ]]; then
  node "$PROJECT_ROOT/scripts/package-processes.mjs" "$APP"
fi

if [[ "$PROJECT_ROOT" != "$BUILD_ROOT" ]]; then
  node "$PROJECT_ROOT/scripts/stage-build.mjs"
fi
cd "$BUILD_ROOT"
npm ci --silent --no-audit
npm --prefix apps/desktop ci --silent --no-audit
node apps/desktop/scripts/prepare-electron-runtime.mjs
if [[ "$PROFILE" == "release" ]]; then
  npm --prefix apps/desktop run package:release
else
  if [[ "$LOCAL_TEST" == "1" ]]; then
    npm --prefix apps/desktop run package:local
  else
    npm --prefix apps/desktop run package
  fi
  SELF_CHECK_LOG="$OUTPUT_DIRECTORY/headless-check.stdout.log"
  "$EXECUTABLE" --personal-smoke-check | tee "$SELF_CHECK_LOG"
  node "$PROJECT_ROOT/apps/desktop/scripts/verify-personal-smoke.mjs" "$SELF_CHECK_LOG"
  node "$PROJECT_ROOT/scripts/package-processes.mjs" "$APP" --wait-ms=15000
  if [[ "$LAUNCH" == "1" ]]; then
    open "$APP"
    for ((attempt=0; attempt<20; attempt++)); do
      if pgrep -f "$EXECUTABLE" >/dev/null; then
        echo "已启动统一测试包：$APP"
        exit 0
      fi
      sleep 1
    done
    echo "系统未启动统一测试包：$APP" >&2
    exit 1
  fi
fi
