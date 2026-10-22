#Requires -Version 5.1
# Windows 原生 sidecar 准备入口，与 script/prepare_sidecar.sh（macOS 侧）语义对齐但不共用
# 实现：按词来工作台 Windows 端适配技术方案第 10 节，准备 App Server 与 Codex Harness
# 两个 externalBin sidecar，并为 Codex Harness 准备 Windows 沙箱 helper 资源。
#
# 2026-09-26 已在 Windows 10 22H2 x64 原生编译并校验全部随包程序；
# 证据见 07测试/03测试报告/Windows原生核验-2026-09-26。

param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "debug"
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows-build-environment.ps1')
if (-not (Test-Path -LiteralPath (Join-Path $MsvcRoot 'VC\Auxiliary\Build\vcvars64.bat'))) {
    throw "微软 C++ Build Tools 尚未就绪，请先安装或配置个人版工具链：$MsvcRoot"
}
$TargetTriple = "x86_64-pc-windows-msvc"

if ($ProjectRoot -ne $BuildRoot) {
    Write-Error "sidecar 只能由统一构建目录中的 package.ps1 调用（期望 $BuildRoot，实际 $ProjectRoot）"
    exit 1
}

Set-Location $ProjectRoot

function Get-ExecutableSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    # 嵌套 npm/PowerShell 进程中不依赖 Get-FileHash 的模块自动加载。
    # 直接使用 .NET 流式 SHA-256，读取或计算失败仍终止构建。
    $Hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $Stream = [System.IO.File]::OpenRead($Path)
        try {
            return [System.BitConverter]::ToString($Hasher.ComputeHash($Stream)).Replace("-", "")
        } finally {
            $Stream.Dispose()
        }
    } finally {
        $Hasher.Dispose()
    }
}

function Copy-VerifiedExecutable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Source,
        [Parameter(Mandatory = $true)]
        [string]$Destination,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "$Label 构建产物不存在：$Source"
    }
    if ((Get-Item -LiteralPath $Source).Length -le 0) {
        throw "$Label 构建产物为空：$Source"
    }

    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        throw "$Label 复制失败：$Destination"
    }
    if ((Get-Item -LiteralPath $Destination).Length -le 0) {
        throw "$Label 复制后文件为空：$Destination"
    }

    $SourceHash = Get-ExecutableSha256 -Path $Source
    $DestinationHash = Get-ExecutableSha256 -Path $Destination
    if ($SourceHash -ne $DestinationHash) {
        throw "$Label 复制后校验失败：$Destination"
    }
}

if ($BuildProfile -eq "debug") {
    cargo build -p contexthub-app-server -p cilai-host-service --target $TargetTriple
} else {
    cargo build -p contexthub-app-server -p cilai-host-service --target $TargetTriple --release
}
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 按构建配置分目录：两种配置此前写同一个路径，后跑的覆盖先跑的。以正式配置暂存时
# 取到的可能是调试配置留下的产物，而两者文件名一致，谁也看不出来。
$SidecarDir = Join-Path $ProjectRoot "target\sidecar\$BuildProfile"
$CodexResourcesDir = Join-Path $SidecarDir "codex-runtime/codex-resources"
New-Item -ItemType Directory -Force -Path $SidecarDir | Out-Null
& node (Join-Path $ProjectRoot "scripts/prepare-personal-node.mjs") $SidecarDir "cilai-node-$TargetTriple.exe"
if ($LASTEXITCODE -ne 0) { throw "个人能力运行时暂存失败" }
& node (Join-Path $ProjectRoot "scripts/prepare-git-runtime.mjs") $SidecarDir
if ($LASTEXITCODE -ne 0) { throw "Git 运行时暂存失败" }

$AppServerSource = Join-Path $ProjectRoot "target\$TargetTriple\$BuildProfile\contexthub-app-server.exe"
$AppServerTarget = Join-Path $SidecarDir "cilai-app-server-$TargetTriple.exe"
Copy-VerifiedExecutable -Source $AppServerSource -Destination $AppServerTarget -Label "App Server sidecar"

# 官方 Codex 发行包逐字保留；校验归档和程序哈希，保留第三方源码快照。
& node (Join-Path $ProjectRoot "scripts/prepare-codex-runtime.mjs") $SidecarDir
if ($LASTEXITCODE -ne 0) { throw "官方 Codex 发行包准备失败" }
# codex-runtime 保留 bin、codex-path、codex-resources 及官方包清单。
$CodexTarget = Join-Path $SidecarDir "codex-runtime/bin/codex-app-server.exe"
Write-Output "Windows sidecar 准备完成：$AppServerTarget, $CodexTarget"
Write-Output "Windows 沙箱 helper 准备完成：$CodexResourcesDir"
