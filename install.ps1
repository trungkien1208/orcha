[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repository = "trungkien1208/orcha"
$assetName = "orcha-windows-x86_64.zip"
$releaseApi = "https://api.github.com/repos/$repository/releases/latest"
$release = Invoke-RestMethod -Uri $releaseApi -Headers @{ "User-Agent" = "orcha-installer" }
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if ($null -eq $asset) {
    throw "The latest ORCHA release does not contain $assetName yet."
}

$tag = [string]$release.tag_name
if ([string]::IsNullOrWhiteSpace($tag) -or $tag -notmatch '^orcha-[A-Za-z0-9._-]+$') {
    throw "The latest ORCHA release has an invalid tag '$tag'."
}

$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$installRoot = Join-Path $localAppData "Programs\Orcha"
$releasesRoot = Join-Path $installRoot "releases"
$binDirectory = Join-Path $installRoot "bin"
$releaseDirectory = Join-Path $releasesRoot $tag
$activeFile = Join-Path $binDirectory "active.txt"

New-Item -ItemType Directory -Path $releasesRoot, $binDirectory -Force | Out-Null

if (-not (Test-Path -LiteralPath (Join-Path $releaseDirectory "herdr.exe") -PathType Leaf)) {
    $workDirectory = Join-Path ([IO.Path]::GetTempPath()) ("orcha-install-" + [guid]::NewGuid().ToString("N"))
    $archivePath = Join-Path $workDirectory $assetName
    $extractDirectory = Join-Path $workDirectory "package"
    $stagingDirectory = Join-Path $releasesRoot (".staging-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $workDirectory, $extractDirectory, $stagingDirectory -Force | Out-Null

    try {
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archivePath -Headers @{ "User-Agent" = "orcha-installer" }

        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
        try {
            $extractRoot = [IO.Path]::GetFullPath($extractDirectory).TrimEnd('\') + '\'
            foreach ($entry in $archive.Entries) {
                $entryPath = $entry.FullName.Replace('/', '\')
                if ([IO.Path]::IsPathRooted($entryPath) -or
                    $entryPath -match '^[A-Za-z]:' -or
                    ($entryPath -split '\\' -contains '..')) {
                    throw "The ORCHA package contains an unsafe path '$($entry.FullName)'."
                }
                $destination = [IO.Path]::GetFullPath((Join-Path $extractDirectory $entryPath))
                if ($destination.Equals($extractRoot.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
                    continue
                }
                if (-not $destination.StartsWith($extractRoot, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "The ORCHA package contains an unsafe path '$($entry.FullName)'."
                }
            }
        } finally {
            $archive.Dispose()
        }

        Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDirectory -Force
        $binary = Get-ChildItem -LiteralPath $extractDirectory -Filter "herdr.exe" -File -Recurse |
            Select-Object -First 1
        if ($null -eq $binary) {
            throw "The ORCHA package does not contain herdr.exe."
        }
        $packageDirectory = $binary.Directory.FullName
        if (-not (Test-Path -LiteralPath (Join-Path $packageDirectory "conpty") -PathType Container)) {
            throw "The ORCHA package is missing its ConPTY runtime."
        }

        Copy-Item -Path (Join-Path $packageDirectory "*") -Destination $stagingDirectory -Recurse -Force
        if (Test-Path -LiteralPath $releaseDirectory) {
            Remove-Item -LiteralPath $releaseDirectory -Recurse -Force
        }
        Move-Item -LiteralPath $stagingDirectory -Destination $releaseDirectory
    } finally {
        if (Test-Path -LiteralPath $workDirectory) {
            Remove-Item -LiteralPath $workDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $stagingDirectory) {
            Remove-Item -LiteralPath $stagingDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

$launcher = @'
@echo off
setlocal
set "ORCHA_APP=1"
set "ORCHA_RELEASE="
set /p "ORCHA_RELEASE="<"%~dp0active.txt"
if not defined ORCHA_RELEASE (
  echo orcha is not installed correctly. Run the Windows installer again.
  exit /b 1
)
set "ORCHA_EXE=%~dp0..\releases\%ORCHA_RELEASE%\herdr.exe"
if not exist "%ORCHA_EXE%" (
  echo The active orcha release is missing. Run the Windows installer again.
  exit /b 1
)
"%ORCHA_EXE%" %*
exit /b %ERRORLEVEL%
'@
Set-Content -LiteralPath (Join-Path $binDirectory "orcha.cmd") -Value $launcher -Encoding Ascii

$activeTemporaryFile = "$activeFile.$PID.tmp"
Set-Content -LiteralPath $activeTemporaryFile -Value $tag -Encoding Ascii
Move-Item -LiteralPath $activeTemporaryFile -Destination $activeFile -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
$pathAlreadyContainsOrcha = $false
foreach ($entry in $pathEntries) {
    try {
        $entryPath = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"'))).TrimEnd('\')
        $binPath = [IO.Path]::GetFullPath($binDirectory).TrimEnd('\')
        if ($entryPath.Equals($binPath, [StringComparison]::OrdinalIgnoreCase)) {
            $pathAlreadyContainsOrcha = $true
            break
        }
    } catch {
        continue
    }
}
if (-not $pathAlreadyContainsOrcha) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $binDirectory } else { "$binDirectory;$userPath" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
}

Write-Host "ORCHA $tag is installed. Open a new terminal and run: orcha"
