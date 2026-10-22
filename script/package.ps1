param(
    [switch]$Release,
    [switch]$Portable,
    [switch]$Launch
)

$ErrorActionPreference = "Stop"
if ($Release -or $Portable) { throw "个人版当前只允许本地测试包；发布身份和更新源尚未配置" }
if ($Release -and $Launch) { throw "正式分发构建不通过开发启动入口运行" }
if ($Portable -and ($Release -or $Launch)) { throw "免安装更新包使用独立打包模式" }

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows-build-environment.ps1')
if (-not (Test-Path -LiteralPath (Join-Path $MsvcRoot 'VC\Auxiliary\Build\vcvars64.bat'))) {
    throw "微软 C++ Build Tools 尚未就绪，请先安装或配置个人版工具链：$MsvcRoot"
}
$NodeVersion = node --version
if ($NodeVersion -ne "v24.18.0") {
    throw "打包需要 Node 24.18.0，当前为 $NodeVersion；请先切换 Node 版本"
}

if (-not $env:CILAI_BUILD_LOCK_TOKEN) {
    $LockArguments = @()
    if ($Release) { $LockArguments += "-Release" }
    if ($Portable) { $LockArguments += "-Portable" }
    if ($Launch) { $LockArguments += "-Launch" }
    & node (Join-Path $ProjectRoot 'scripts\build-lock.mjs') -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @LockArguments
    exit $LASTEXITCODE
}
& node (Join-Path $ProjectRoot 'scripts\build-lock.mjs') --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$OutputChannel = & node (Join-Path $ProjectRoot 'scripts\personal-build.mjs') --channel
if ($LASTEXITCODE -ne 0) { throw '个人版本地构建通道无效' }
$OutputJson = & node (Join-Path $ProjectRoot 'apps\desktop\scripts\package-output.mjs') $OutputChannel --json
if ($LASTEXITCODE -ne 0) { throw '无法确定构建通道输出目录' }
$OutputDirectory = $OutputJson | ConvertFrom-Json
$PackageDirectory = Join-Path $OutputDirectory 'win-unpacked'
$PackagePrefix = [System.IO.Path]::GetFullPath($PackageDirectory).TrimEnd([char[]]@('\', '/')) + '\'
$Executable = Join-Path $PackageDirectory 'cilai-personal-workbench.exe'
function Get-PackagedProcesses {
    Get-CimInstance Win32_Process | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($PackagePrefix, [StringComparison]::OrdinalIgnoreCase)
    }
}
$Running = Get-PackagedProcesses
if ($Running) {
    throw "统一测试包仍有随包进程运行。请完成正常退出后重新打包：$PackageDirectory"
}

if ($ProjectRoot -ne $BuildRoot) {
    & node "$ProjectRoot\scripts\stage-build.mjs"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
Set-Location $BuildRoot
Install-WindowsBuildDependencies

if ($Release) {
    & npm --prefix apps/desktop run package:release
    exit $LASTEXITCODE
}
if ($Portable) {
    & npm --prefix apps/desktop run package:portable
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} else {
    & npm --prefix apps/desktop run package
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
$CheckDirectory = Split-Path -Parent (Split-Path -Parent $Executable)
$CheckOutput = Join-Path $CheckDirectory "headless-check.stdout.log"
$CheckError = Join-Path $CheckDirectory "headless-check.stderr.log"
# GUI 子系统程序直接调用可能立即返回且不更新 LASTEXITCODE；等待自检主进程，
# 避免 Start-Process -Wait 因仍在运行的子进程而无法结束构建入口。
$CheckUserData = Join-Path $BuildRoot ('tmp\desktop-headless-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $CheckUserData | Out-Null
$CheckLocalData = Join-Path $CheckUserData 'local'
$CheckRoamingData = Join-Path $CheckUserData 'roaming'
New-Item -ItemType Directory -Path $CheckLocalData, $CheckRoamingData | Out-Null
$CheckArguments = @('--personal-smoke-check', ('"--user-data-dir=' + (Join-Path $CheckUserData 'shell') + '"'))
# Electron 与原生业务服务使用不同的数据根；两者都隔离，避免自检读取当前员工的运行状态。
$PreviousCheckLocalData = $env:LOCALAPPDATA
$PreviousCheckRoamingData = $env:APPDATA
try {
    $env:LOCALAPPDATA = $CheckLocalData
    $env:APPDATA = $CheckRoamingData
    $CheckProcess = Start-Process -FilePath $Executable -ArgumentList $CheckArguments -WindowStyle Hidden -PassThru -RedirectStandardOutput $CheckOutput -RedirectStandardError $CheckError
    # Windows PowerShell 5.1 必须提前保留句柄，否则等待结束后 ExitCode 可能为空，
    # exit $null 会把自检失败或未完成启动错误地报告为成功。
    $null = $CheckProcess.Handle
} finally {
    $env:LOCALAPPDATA = $PreviousCheckLocalData
    $env:APPDATA = $PreviousCheckRoamingData
}
if (-not $CheckProcess.WaitForExit(120000)) {
    Stop-Process -Id $CheckProcess.Id -Force -ErrorAction SilentlyContinue
    throw "统一测试包启动自检超过 120 秒：$Executable"
}
Get-Content -LiteralPath $CheckOutput -Encoding UTF8
Get-Content -LiteralPath $CheckError -Encoding UTF8
$CheckExitCode = $CheckProcess.ExitCode
if ($null -eq $CheckExitCode) { throw "启动自检退出码不可用，不能确认构建成功：$Executable" }
if ($CheckExitCode -ne 0) { exit $CheckExitCode }
& node (Join-Path $ProjectRoot 'apps\desktop\scripts\verify-personal-smoke.mjs') $CheckOutput
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$ExitDeadline = [DateTime]::UtcNow.AddSeconds(15)
while (Get-PackagedProcesses) {
    if ([DateTime]::UtcNow -ge $ExitDeadline) { throw '启动自检结束后仍有随包进程未完成收尾' }
    Start-Sleep -Milliseconds 100
}
if ($Launch) {
    Start-Process -FilePath $Executable
    for ($Attempt = 0; $Attempt -lt 20; $Attempt++) {
        $Running = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $Executable }
        if ($Running) {
            Write-Output "已启动统一测试包：$Executable"
            exit 0
        }
        Start-Sleep -Seconds 1
    }
    throw "系统未启动统一测试包：$Executable"
}
