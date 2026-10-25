#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
$SourceRoot = Split-Path -Parent $PSScriptRoot
$BuildRoot = & node (Join-Path $SourceRoot 'scripts\personal-build.mjs')
if ($LASTEXITCODE -ne 0) { throw '个人版构建根校验失败' }
$env:CILAI_BUILD_ROOT = $BuildRoot
$env:CARGO_TARGET_DIR = Join-Path $BuildRoot 'target'
$env:CARGO_HOME = Join-Path $BuildRoot 'toolchains\cargo'
$env:CARGO_BUILD_JOBS = '2'
$CacheRoot = Join-Path $BuildRoot 'cache'
$env:npm_config_cache = Join-Path $CacheRoot 'npm'
$env:ELECTRON_CACHE = Join-Path $CacheRoot 'electron'
$env:electron_config_cache = Join-Path $env:ELECTRON_CACHE 'Cache'
$env:ELECTRON_BUILDER_CACHE = Join-Path $CacheRoot 'electron-builder'
$ConfigRoot = Join-Path $BuildRoot 'config'
New-Item -ItemType Directory -Force -Path $ConfigRoot | Out-Null
$env:npm_config_userconfig = Join-Path $ConfigRoot 'npmrc'
$env:npm_config_globalconfig = Join-Path $ConfigRoot 'npmrc-global'
foreach ($ConfigFile in @($env:npm_config_userconfig, $env:npm_config_globalconfig)) {
    if (-not (Test-Path -LiteralPath $ConfigFile)) { New-Item -ItemType File -Path $ConfigFile | Out-Null }
}
if ($env:CILAI_BUILD_RUN_TMP) { $env:TEMP = $env:CILAI_BUILD_RUN_TMP } else { $env:TEMP = Join-Path $BuildRoot 'tmp' }
$env:TMP = $env:TEMP
$MsvcRoot = $env:CILAI_PERSONAL_MSVC_ROOT
if (-not $MsvcRoot) {
    $VsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $VsWhere) {
        $MsvcRoot = & $VsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    }
}
if (-not $MsvcRoot) { throw '未找到微软 C++ Build Tools；请安装或设置 CILAI_PERSONAL_MSVC_ROOT' }
foreach ($Directory in @($env:TEMP, $env:npm_config_cache, $env:electron_config_cache, $env:ELECTRON_BUILDER_CACHE)) {
    New-Item -ItemType Directory -Force -Path $Directory | Out-Null
}
function Install-WindowsBuildDependencies {
    & node (Join-Path $SourceRoot 'scripts\personal-build.mjs')
    if ($LASTEXITCODE -ne 0) { throw '个人版构建根校验失败' }
    Push-Location $BuildRoot
    try {
        & node scripts/ensure-build-dependencies.mjs --ci
        if ($LASTEXITCODE -ne 0) { throw "依赖安装或复用校验失败：$LASTEXITCODE" }
        & node apps/desktop/scripts/prepare-electron-runtime.mjs
        if ($LASTEXITCODE -ne 0) { throw "Electron 运行时准备失败：$LASTEXITCODE" }
    } finally { Pop-Location }
}
