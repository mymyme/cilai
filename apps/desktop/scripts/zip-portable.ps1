param(
    [Parameter(Mandatory = $true)][string]$SourceDirectory,
    [Parameter(Mandatory = $true)][string]$ArchivePath
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
# CreateNew 保证本次临时归档不复用旧文件；完整写入后由调用方原子替换本地输出。
$Root = [IO.Path]::GetFullPath($SourceDirectory).TrimEnd('\')
$Stream = [IO.File]::Open($ArchivePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
$Zip = [IO.Compression.ZipArchive]::new($Stream, [IO.Compression.ZipArchiveMode]::Create, $true, [Text.Encoding]::UTF8)
try {
    foreach ($File in [IO.Directory]::EnumerateFiles($Root, '*', [IO.SearchOption]::AllDirectories)) {
        # Windows PowerShell 5.1 的 CreateFromDirectory 会写反斜杠，显式使用 ZIP 标准路径。
        $Name = $File.Substring($Root.Length + 1).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($Zip, $File, $Name, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally { $Zip.Dispose(); $Stream.Dispose() }
