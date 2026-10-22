param(
    [Parameter(Mandatory = $true)][string]$Handoff,
    [ValidateRange(3, 180)][int]$HealthTimeoutSeconds = 90
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression

function LongPath([string]$Value) {
    if ($Value.StartsWith('\\?\')) { return $Value }
    if ($Value.StartsWith('\\')) { return '\\?\UNC\' + $Value.Substring(2) }
    return '\\?\' + $Value
}
function FullPath([string]$Value) { return [IO.Path]::GetFullPath($Value).TrimEnd('\') }
function Under([string]$Value, [string]$Root) {
    return (FullPath $Value).StartsWith((FullPath $Root) + '\', [StringComparison]::OrdinalIgnoreCase)
}
function OrdinaryTree([string]$Root) {
    $Attributes = [IO.File]::GetAttributes((LongPath $Root))
    if ($Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "更新目录含重解析点：$Root" }
    if ($Attributes -band [IO.FileAttributes]::Directory) {
        foreach ($Child in [IO.Directory]::EnumerateFileSystemEntries((LongPath $Root))) { OrdinaryTree $Child }
    }
}
function Digest([string]$File, [string]$Algorithm = 'SHA256') {
    $Hasher = [Security.Cryptography.HashAlgorithm]::Create($Algorithm)
    $Stream = [IO.File]::OpenRead((LongPath $File))
    try { return [BitConverter]::ToString($Hasher.ComputeHash($Stream)).Replace('-', '').ToLowerInvariant() }
    finally { $Stream.Dispose(); $Hasher.Dispose() }
}
function SafeRelative([string]$Name) {
    if (-not $Name -or $Name.Contains('\') -or $Name.StartsWith('/') -or $Name.Contains(':')) { throw "更新包路径无效：$Name" }
    $Trimmed = $Name.TrimEnd('/')
    foreach ($Part in $Trimmed.Split('/')) {
        $Stem = $Part.Split('.')[0].ToUpperInvariant()
        if (-not $Part -or $Part -eq '.' -or $Part -eq '..' -or $Part -ne $Part.Trim() -or $Part.EndsWith('.') -or
            $Part -match '[<>"|?*\x00-\x1f]' -or $Stem -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$') { throw "更新包路径无效：$Name" }
    }
    return $Trimmed
}
function MoveSibling([string]$Source, [string]$Destination) {
    # 每次移动前核对绝对路径，只移动这次更新明确命名的相邻程序目录。
    $Allowed = @($script:Target, $script:Stage, $script:Backup, $script:Failed)
    $From = FullPath $Source; $To = FullPath $Destination
    if ($Allowed -notcontains $From -or $Allowed -notcontains $To -or
        [IO.Path]::GetDirectoryName($From) -ne [IO.Path]::GetDirectoryName($script:Target) -or
        [IO.Path]::GetDirectoryName($To) -ne [IO.Path]::GetDirectoryName($script:Target)) { throw '更新目录移动范围不匹配' }
    for ($Attempt = 0; $Attempt -lt 80; $Attempt++) {
        try { [IO.Directory]::Move((LongPath $From), (LongPath $To)); return }
        catch [IO.IOException] { if ($Attempt -eq 79) { throw }; Start-Sleep -Milliseconds 250 }
    }
}
function SaveResult([string]$Status, [string]$Message) {
    $Value = @{ status = $Status; message = $Message; version = $script:UpdateRequest.version; target = $script:Target; backup = $script:Backup }
    [IO.File]::WriteAllText((Join-Path $script:Ticket 'result.json'), ($Value | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
}
function StartClient([bool]$Updating) {
    $Executable = Join-Path $script:Target $script:UpdateRequest.executableName
    $Flag = if ($Updating) { '--cilai-update=' } else { '--cilai-update-rollback=' }
    $Arguments = @($Flag + $script:UpdateRequest.id)
    $Arguments += ('"--user-data-dir=' + $script:UpdateRequest.userData + '"')
    # 自检由独立测试脚本指定；普通员工重启使用应用原有窗口行为。
    if ($script:UpdateRequest.testHeadless -eq $true) { $Arguments += '--headless-check' }
    $WindowStyle = if ($script:UpdateRequest.testHeadless -eq $true) { 'Hidden' } else { 'Normal' }
    return Start-Process -FilePath $Executable -WorkingDirectory $script:Target -ArgumentList $Arguments -WindowStyle $WindowStyle -PassThru
}

$MovedOld = $false
$MovedNew = $false
$NewProcess = $null
$VerifiedTarget = $false
try {
    $UpdateRequest = Get-Content -LiteralPath $Handoff -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($UpdateRequest.schemaVersion -ne 1 -or $UpdateRequest.id -notmatch '^[a-f0-9-]{36}$' -or $UpdateRequest.nonce -notmatch '^[a-f0-9-]{36}$' -or
        $UpdateRequest.version -notmatch '^\d+\.\d+\.\d+$' -or $UpdateRequest.executableName -ne '词来工作台.exe') { throw '更新交接信息无效' }
    $Ticket = FullPath $UpdateRequest.ticket
    if ($Ticket -ne (FullPath $PSScriptRoot) -or $Ticket -ne (FullPath ([IO.Path]::GetDirectoryName($Handoff))) -or
        [IO.Path]::GetFileName($Ticket) -ne $UpdateRequest.id) { throw '更新交接目录不匹配' }
    OrdinaryTree $Ticket
    $Target = FullPath $UpdateRequest.target
    $ProfileDirectory = FullPath $UpdateRequest.userData
    if ((Under $ProfileDirectory $Target) -or $ProfileDirectory -eq $Target) { throw '用户数据目录不能位于待替换程序目录中' }
    if ($Target -eq [IO.Path]::GetPathRoot($Target).TrimEnd('\') -or (Under $Ticket $Target) -or (Under $Target $Ticket)) { throw '程序与更新缓存目录范围无效' }
    $Stage = FullPath ($Target + '.cilai-stage-' + $UpdateRequest.id)
    $Backup = FullPath ($Target + '.cilai-backup-' + $UpdateRequest.id)
    $Failed = FullPath ($Target + '.cilai-failed-' + $UpdateRequest.id)
    OrdinaryTree $Target
    $Config = [IO.File]::ReadAllText((LongPath (Join-Path $Target 'resources\portable-update.json'))) | ConvertFrom-Json
    if ($Config.kind -ne 'cilai-portable' -or $Config.schemaVersion -ne 1) { throw '当前目录不是免安装更新包' }
    $VerifiedTarget = $true
    # Windows 的进程工作目录会占用目录句柄。即使调用方从旧程序目录启动助手，
    # 也必须在移动之前切离；PowerShell 位置与进程工作目录分别设置。
    Set-Location -LiteralPath $Ticket
    [Environment]::CurrentDirectory = $Ticket
    $Started = @{ id = $UpdateRequest.id; nonce = $UpdateRequest.nonce; version = $UpdateRequest.version } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $Ticket 'started.tmp'), $Started, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move((Join-Path $Ticket 'started.tmp'), (Join-Path $Ticket 'started.json'))
    $ArchivePath = FullPath $UpdateRequest.archive
    if (-not (Under $ArchivePath ([IO.Path]::GetDirectoryName($Ticket)))) { throw '更新文件不在本次缓存范围内' }
    OrdinaryTree $ArchivePath
    if ([IO.Directory]::Exists((LongPath $Stage)) -or [IO.Directory]::Exists((LongPath $Backup)) -or [IO.Directory]::Exists((LongPath $Failed))) { throw '更新暂存目录已经存在' }
    $Stream = [IO.File]::Open((LongPath $ArchivePath), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($Stream.Length -ne $UpdateRequest.size -or $Stream.Length -gt 2147483648) { throw '更新文件长度不匹配' }
        $Hasher = [Security.Cryptography.SHA512]::Create()
        try { $Hash = [Convert]::ToBase64String($Hasher.ComputeHash($Stream)) } finally { $Hasher.Dispose() }
        if ($Hash -cne $UpdateRequest.sha512) { throw '更新文件摘要不匹配' }
        $Stream.Position = 0
        $Zip = [IO.Compression.ZipArchive]::new($Stream, [IO.Compression.ZipArchiveMode]::Read, $true)
        try {
            $Names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            $Files = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            $Expanded = [long]0
            if ($Zip.Entries.Count -gt 10000) { throw '更新包条目过多' }
            foreach ($Entry in $Zip.Entries) {
                $Name = SafeRelative $Entry.FullName
                if (-not $Names.Add($Name.Normalize())) { throw '更新包存在重名路径' }
                $Type = ($Entry.ExternalAttributes -shr 16) -band 61440
                if ($Type -eq 40960 -or ($Entry.ExternalAttributes -band 1024)) { throw '更新包不允许符号链接或重解析点' }
                $Expanded += $Entry.Length
                if ($Entry.Length -gt 1073741824 -or $Expanded -gt 3221225472) { throw '更新包解压大小超限' }
            }
            [IO.Directory]::CreateDirectory((LongPath $Stage)) | Out-Null
            foreach ($Entry in $Zip.Entries) {
                $Name = SafeRelative $Entry.FullName
                $Destination = FullPath (Join-Path $Stage $Name.Replace('/', '\'))
                if (-not (Under $Destination $Stage)) { throw '更新包路径越界' }
                if ($Entry.FullName.EndsWith('/')) { [IO.Directory]::CreateDirectory((LongPath $Destination)) | Out-Null; continue }
                [IO.Directory]::CreateDirectory((LongPath ([IO.Path]::GetDirectoryName($Destination)))) | Out-Null
                $Source = $Entry.Open()
                $Output = [IO.File]::Open((LongPath $Destination), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                try {
                    $Buffer = [byte[]]::new(65536)
                    $Written = [long]0
                    while (($Read = $Source.Read($Buffer, 0, $Buffer.Length)) -gt 0) {
                        $Written += $Read
                        if ($Written -gt $Entry.Length) { throw '更新包实际解压大小超过声明长度' }
                        $Output.Write($Buffer, 0, $Read)
                    }
                    if ($Written -ne $Entry.Length) { throw '更新包解压长度不匹配' }
                } finally { $Output.Dispose(); $Source.Dispose() }
                $Files.Add($Name) | Out-Null
            }
        } finally { $Zip.Dispose() }
    } finally { $Stream.Dispose() }
    $PackageFile = Join-Path $Stage 'portable-package.json'
    if ((Digest $PackageFile) -cne $UpdateRequest.packageManifestSha256) { throw '更新包内部清单摘要不匹配' }
    $Package = [IO.File]::ReadAllText((LongPath $PackageFile)) | ConvertFrom-Json
    if ($Package.schemaVersion -ne 1 -or $Package.kind -ne 'cilai-portable' -or $Package.version -ne $UpdateRequest.version -or $Package.files.Count -ne ($Files.Count - 1)) { throw '更新包内部清单无效' }
    $Listed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($Record in $Package.files) {
        $Name = SafeRelative $Record.path
        if ($Name -eq 'portable-package.json' -or -not $Files.Contains($Name) -or -not $Listed.Add($Name)) { throw '更新包文件集合不匹配' }
        $File = Join-Path $Stage $Name.Replace('/', '\')
        if ([IO.FileInfo]::new((LongPath $File)).Length -ne $Record.size -or (Digest $File) -cne $Record.sha256) { throw "更新包文件校验失败：$Name" }
    }
    if (-not $Listed.Contains($UpdateRequest.executableName) -or -not $Listed.Contains('resources/app.asar') -or -not $Listed.Contains('resources/portable-update.json')) { throw '更新包缺少客户端组件' }
    $OldProcess = $null
    try { $OldProcess = [Diagnostics.Process]::GetProcessById([int]$UpdateRequest.oldPid) } catch [ArgumentException] {}
    if ($null -ne $OldProcess) {
        try {
            if (-not $OldProcess.HasExited -and (FullPath $OldProcess.MainModule.FileName) -eq (FullPath (Join-Path $Target $UpdateRequest.executableName))) {
                if (-not $OldProcess.WaitForExit(120000)) { throw '等待客户端退出超时，旧版本保持不变' }
            }
        } finally { $OldProcess.Dispose() }
    }
    MoveSibling $Target $Backup
    $MovedOld = $true
    MoveSibling $Stage $Target
    $MovedNew = $true
    $NewProcess = StartClient $true
    $Until = [DateTime]::UtcNow.AddSeconds($HealthTimeoutSeconds)
    do {
        $ReadyPath = Join-Path $Ticket 'ready.json'
        if (Test-Path -LiteralPath $ReadyPath) {
            $Ready = Get-Content -LiteralPath $ReadyPath -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($Ready.id -eq $UpdateRequest.id -and $Ready.nonce -eq $UpdateRequest.nonce -and $Ready.version -eq $UpdateRequest.version -and $Ready.pid -eq $NewProcess.Id) {
                SaveResult 'updated' '已完成免安装更新，旧版本备份已保留。'
                exit 0
            }
            throw '新版启动确认不匹配'
        }
        if ($NewProcess.HasExited) { throw '新版客户端启动失败' }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $Until)
    throw '新版客户端启动确认超时'
} catch {
    $Message = $_.Exception.Message
    try {
        if ($null -ne $NewProcess -and -not $NewProcess.HasExited) { $NewProcess.Kill(); $NewProcess.WaitForExit() }
        if ($MovedNew) { MoveSibling $Target $Failed }
        if ($MovedOld) { MoveSibling $Backup $Target; SaveResult 'rolledBack' $Message; StartClient $false | Out-Null }
        elseif ($Ticket) { SaveResult 'failed' $Message; if ($VerifiedTarget) { StartClient $false | Out-Null } }
    } catch { if ($Ticket) { SaveResult 'rollbackFailed' ($Message + '；恢复失败：' + $_.Exception.Message) } }
    Write-Error $Message
    exit 1
}
