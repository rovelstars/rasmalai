# Rasmalai installer for Windows: irm https://rnx.dev/install.ps1 | iex
# Needs PowerShell 5.1+ and tar.exe (ships with Windows 10 1803 and later).
# Knobs: -Version (default: latest), -InstallDir (default: %LOCALAPPDATA%\rnx).
# To remove later: delete $InstallDir and drop its bin/ from the user PATH
# (see the uninstall notes in the guide's troubleshooting chapter).
param(
  [string]$Version = "latest",
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "rnx")
)

$ErrorActionPreference = "Stop"
$Repo = "rovelstars/rasmalai"

if (-not [Environment]::Is64BitOperatingSystem) {
  Write-Error "Rasmalai ships 64-bit Windows builds only."
  exit 1
}
$Target = "x86_64-windows"

if ([string]::IsNullOrEmpty($env:LOCALAPPDATA)) {
  Write-Error "LOCALAPPDATA is not set; pass -InstallDir explicitly."
  exit 1
}

if ($Version -eq "latest") {
  $Base = "https://github.com/$Repo/releases/latest/download"
} else {
  $Base = "https://github.com/$Repo/releases/download/$Version"
}
$Archive = "rnx-$Target.tar.gz"
$Url = "$Base/$Archive"

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("rnx-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Write-Host "downloading $Url"
  Invoke-WebRequest -Uri $Url -OutFile (Join-Path $Tmp $Archive) -UseBasicParsing

  $Tar = Get-Command tar.exe -ErrorAction Stop
  & $Tar.Source -xzf (Join-Path $Tmp $Archive) -C $Tmp
  if ($LASTEXITCODE -ne 0) { throw "unpack failed" }

  $Stage = Join-Path $Tmp ("rnx-" + $Target)
  if (-not (Test-Path (Join-Path $Stage "bin"))) { throw "archive has no bin/ directory" }

  $BinDir = Join-Path $InstallDir "bin"
  New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
  Copy-Item (Join-Path $Stage "bin\*") $BinDir -Recurse -Force

  $Bin = Join-Path $BinDir "rnx.exe"
  if (-not (Test-Path $Bin)) { throw "installed but rnx.exe is missing in $BinDir" }

  $Installed = (& $Bin --version 2>$null) -replace '^rnx\s+', ''
  if ([string]::IsNullOrEmpty($Installed)) { $Installed = $Version }

  & $Bin fetch-std 2>$null | Out-Null
  if ($LASTEXITCODE -ne 0) {
    Write-Warning "stdlib cache seeding failed (offline install? run 'rnx fetch-std' later)"
  } else {
    Write-Host "stdlib cache: seeded"
  }

  $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if (($UserPath -split ';' | Where-Object { $_ -eq $BinDir }).Count -eq 0) {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$BinDir", "User")
    Write-Host "added $BinDir to the user PATH (restart the terminal to pick it up)"
  } else {
    Write-Host "$BinDir is already on your PATH"
  }

  Write-Host "rasmalai installed successfully"
  Write-Host "  binary:   $Bin"
  Write-Host "  version:  $Installed"
} finally {
  Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
