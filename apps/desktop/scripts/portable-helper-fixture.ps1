param(
    [Parameter(Mandatory = $true)][string]$HelperPath,
    [string[]]$Cases = @('success', 'helper-inherits-target', 'helper-inherits-target-child', 'hash-corrupt', 'zip-slip', 'duplicate-path', 'inventory-corrupt', 'wrong-version', 'startup-failed', 'startup-timeout')
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.IO.Compression
$TestRoot = Join-Path ([IO.Path]::GetTempPath()) ('cilai-helper-' + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $TestRoot | Out-Null
$FixtureExe = Join-Path $TestRoot 'fixture.exe'
Add-Type -ReferencedAssemblies System.Web.Extensions -OutputAssembly $FixtureExe -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.IO;
using System.Diagnostics;
using System.Collections.Generic;
using System.Web.Script.Serialization;
public class Fixture {
  public static void Main(string[] args) {
    if (args.Length == 0 || !args[0].StartsWith("--cilai-update=")) return;
    string folder=AppDomain.CurrentDomain.BaseDirectory;
    if(File.Exists(Path.Combine(folder,"fail-startup.txt"))) return;
    if(File.Exists(Path.Combine(folder,"timeout-startup.txt"))) { System.Threading.Thread.Sleep(10000); return; }
    string ticket=Environment.GetEnvironmentVariable("CILAI_HELPER_TEST_TICKET");
    var json=new JavaScriptSerializer();
    var input=json.Deserialize<Dictionary<string,object>>(File.ReadAllText(Path.Combine(ticket,"handoff.json")));
    var ready=new Dictionary<string,object>{{"id",input["id"]},{"nonce",input["nonce"]},{"version",input["version"]},{"pid",Process.GetCurrentProcess().Id}};
    File.WriteAllText(Path.Combine(ticket,"ready.tmp"),json.Serialize(ready));
    File.Move(Path.Combine(ticket,"ready.tmp"),Path.Combine(ticket,"ready.json"));
    System.Threading.Thread.Sleep(3000);
  }
}
'@
function Hash([byte[]]$Bytes, [string]$Algorithm = 'SHA256', [bool]$Base64 = $false) {
    $Digest = [Security.Cryptography.HashAlgorithm]::Create($Algorithm)
    try { $Value = $Digest.ComputeHash($Bytes); if ($Base64) { return [Convert]::ToBase64String($Value) }; return [BitConverter]::ToString($Value).Replace('-', '').ToLowerInvariant() }
    finally { $Digest.Dispose() }
}
$Results = @()
foreach ($CaseName in $Cases) {
    $CaseRoot = Join-Path $TestRoot $CaseName
    $Target = Join-Path $CaseRoot '中文 app'
    $Cache = Join-Path $CaseRoot 'updates'
    $Id = [Guid]::NewGuid().ToString()
    $Ticket = Join-Path $Cache $Id
    New-Item -ItemType Directory -Path (Join-Path $Target 'resources'), $Ticket | Out-Null
    Copy-Item -LiteralPath $FixtureExe -Destination (Join-Path $Target '词来工作台.exe')
    $ConfigBytes = [Text.Encoding]::UTF8.GetBytes('{"schemaVersion":1,"kind":"cilai-portable","platform":"win32","arch":"x64"}')
    [IO.File]::WriteAllBytes((Join-Path $Target 'resources\portable-update.json'), $ConfigBytes)
    [IO.File]::WriteAllText((Join-Path $Target 'old-version.txt'), 'preserved')
    $UserData = Join-Path $CaseRoot 'user-data.txt'
    [IO.File]::WriteAllText($UserData, 'personal data preserved')
    $Files = [ordered]@{ '词来工作台.exe' = [IO.File]::ReadAllBytes($FixtureExe); 'resources/app.asar' = [Text.Encoding]::UTF8.GetBytes('fixture'); 'resources/portable-update.json' = $ConfigBytes }
    if ($CaseName -eq 'startup-failed') { $Files['fail-startup.txt'] = [byte[]]@() }
    if ($CaseName -eq 'startup-timeout') { $Files['timeout-startup.txt'] = [byte[]]@() }
    $Records = @($Files.Keys | ForEach-Object { @{ path = $_; size = $Files[$_].Length; sha256 = (Hash $Files[$_]) } })
    if ($CaseName -eq 'inventory-corrupt') { $Records[1].sha256 = '0' * 64 }
    $PackageVersion = if ($CaseName -eq 'wrong-version') { '9.0.0' } else { '1.2.3' }
    $InventoryBytes = [Text.Encoding]::UTF8.GetBytes((@{ schemaVersion = 1; kind = 'cilai-portable'; version = $PackageVersion; files = $Records } | ConvertTo-Json -Depth 10 -Compress))
    $Files['portable-package.json'] = $InventoryBytes
    $ArchivePath = Join-Path $Cache 'fixture.zip'
    $Stream = [IO.File]::Open($ArchivePath, [IO.FileMode]::CreateNew)
    $Zip = [IO.Compression.ZipArchive]::new($Stream, [IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        foreach ($Name in $Files.Keys) {
            $Entry = $Zip.CreateEntry($Name)
            $Output = $Entry.Open()
            try { $Output.Write($Files[$Name], 0, $Files[$Name].Length) } finally { $Output.Dispose() }
        }
        if ($CaseName -eq 'zip-slip') { $Entry = $Zip.CreateEntry('../escaped.txt'); $Entry.Open().Dispose() }
        if ($CaseName -eq 'duplicate-path') { $Entry = $Zip.CreateEntry('resources/app.asar'); $Entry.Open().Dispose() }
    } finally { $Zip.Dispose(); $Stream.Dispose() }
    $ArchiveBytes = [IO.File]::ReadAllBytes($ArchivePath)
    $Sha512 = Hash $ArchiveBytes 'SHA512' $true
    if ($CaseName -eq 'hash-corrupt') { $Sha512 = [Convert]::ToBase64String([byte[]]::new(64)) }
    $Handoff = @{ schemaVersion = 1; id = $Id; nonce = [Guid]::NewGuid().ToString(); target = $Target; ticket = $Ticket; archive = $ArchivePath;
        sha512 = $Sha512; size = $ArchiveBytes.Length; packageManifestSha256 = (Hash $InventoryBytes); version = '1.2.3'; executableName = '词来工作台.exe'; oldPid = -1; userData = $CaseRoot; testHeadless = $true }
    $HandoffFile = Join-Path $Ticket 'handoff.json'
    [IO.File]::WriteAllText($HandoffFile, ($Handoff | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    $HelperCopy = Join-Path $Ticket 'apply-portable-update.ps1'
    [IO.File]::WriteAllText($HelperCopy, [IO.File]::ReadAllText($HelperPath), [Text.UTF8Encoding]::new($true))
    $env:CILAI_HELPER_TEST_TICKET = $Ticket
    $Arguments = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $HelperCopy + '"'), '-Handoff', ('"' + $HandoffFile + '"'), '-HealthTimeoutSeconds', '3')
    $HelperWorkingDirectory = switch ($CaseName) {
        'helper-inherits-target' { $Target }
        'helper-inherits-target-child' { Join-Path $Target 'resources' }
        default { $CaseRoot }
    }
    $Process = Start-Process -FilePath powershell.exe -ArgumentList $Arguments -WorkingDirectory $HelperWorkingDirectory -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $CaseRoot 'stdout.log') -RedirectStandardError (Join-Path $CaseRoot 'stderr.log')
    if (-not $Process.WaitForExit(30000)) { $Process.Kill(); throw "助手测试超时：$CaseName" }
    $Result = Get-Content -LiteralPath (Join-Path $Ticket 'result.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $Succeeds = $CaseName -in @('success', 'helper-inherits-target', 'helper-inherits-target-child')
    $Expected = if ($Succeeds) { 'updated' } elseif ($CaseName.StartsWith('startup-')) { 'rolledBack' } else { 'failed' }
    if ($Result.status -ne $Expected) { throw "助手测试失败 $CaseName：预期 $Expected，实际 $($Result.status)，$($Result.message)" }
    if ($Succeeds) {
        if (Test-Path -LiteralPath (Join-Path $Target 'old-version.txt')) { throw '新版目录未生效' }
        if (-not (Test-Path -LiteralPath (Join-Path $Result.backup 'old-version.txt'))) { throw '旧版本备份丢失' }
        if (Test-Path -LiteralPath ($Target + '.cilai-stage-' + $Id)) { throw '更新成功后暂存目录仍在' }
    } elseif ([IO.File]::ReadAllText((Join-Path $Target 'old-version.txt')) -ne 'preserved') { throw '旧版目录未恢复' }
    if ([IO.File]::ReadAllText($UserData) -ne 'personal data preserved') { throw '用户数据被修改' }
    if (Test-Path -LiteralPath (Join-Path $CaseRoot 'escaped.txt')) { throw '解压路径逃逸' }
    $Results += @{ name = $CaseName; status = $Result.status; passed = $true; message = $Result.message }
}
@{ root = $TestRoot; results = $Results } | ConvertTo-Json -Depth 10
