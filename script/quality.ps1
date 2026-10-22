# Windows 原生质量门禁入口，与 script/quality.sh（macOS 侧）步骤顺序一致：工具链检查、
# 依赖漏洞扫描、类型检查、前端测试、前端构建、sidecar 准备、Rust 格式/静态检查/严格
# Clippy/测试、协议生成物一致性检查、cargo-deny。
#
# 尚未在 Windows 实机执行验证；语法与路径按 PowerShell 惯用写法编写，实机验证前不作为
# Windows 质量门禁通过的证据。

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows-build-environment.ps1')
$RequiredNodeVersion = "v24.18.0"

Set-Location $ProjectRoot

$NodeVersion = (node --version 2>$null)
if ($NodeVersion -ne $RequiredNodeVersion) {
    Write-Error "正式校验需要 Node $($RequiredNodeVersion.TrimStart('v'))，当前为 $NodeVersion；请通过 nvm-windows 或官方安装包切换"
    exit 1
}

if (-not $env:CILAI_BUILD_LOCK_TOKEN) {
    & node (Join-Path $ProjectRoot 'scripts\build-lock.mjs') -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath
    exit $LASTEXITCODE
}
& node (Join-Path $ProjectRoot 'scripts\build-lock.mjs') --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (-not $env:CILAI_STAGED_BUILD -and $ProjectRoot -ne $BuildRoot) {
    Write-Output "[stage] 统一在 $BuildRoot 校验"
    & node "$ProjectRoot\scripts\stage-build.mjs"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Push-Location $BuildRoot
    Install-WindowsBuildDependencies
    Pop-Location
    $env:CILAI_STAGED_BUILD = "1"
    & "$BuildRoot\script\quality.ps1"
    exit $LASTEXITCODE
}

npm run check:toolchain
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

npm audit --audit-level=high
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

npm run typecheck
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

npm test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

npm run build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& "$ProjectRoot\script\prepare_sidecar.ps1" -BuildProfile debug
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
node apps/desktop/scripts/stage-binaries.mjs --profile debug
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo check --workspace --all-targets
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo test --workspace
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo run --quiet -p contexthub-protocol --bin generate-contracts -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$DenyVersion = (cargo deny --version)
if ($DenyVersion -ne "cargo-deny 0.20.2") {
    Write-Error "cargo-deny 版本不匹配：要求 0.20.2，当前 $DenyVersion"
    exit 1
}

cargo deny check advisories licenses bans sources
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }


Write-Output "quality.ps1 通过"
