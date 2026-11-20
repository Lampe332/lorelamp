# Builds the release files of Lorelamp: the Windows installer (NSIS, through the Tauri CLI) and a
# portable EXE, named after the version, plus SHA256SUMS.txt. Publishes nothing (no push, tag or
# GitHub release); see docs/releasing.md for the whole flow.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 [-OutDir <folder>]
#
# Default output: target\release-assets\<version>\ in this repo.
[CmdletBinding()]
param([string]$OutDir)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

function Invoke-Step([string]$what, [scriptblock]$run) {
    Write-Host "== $what"
    & $run
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}

# 1. One version everywhere.
$conf = Get-Content (Join-Path $repo 'apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = $conf.version
$product = $conf.productName
$found = [ordered]@{
    'tauri.conf.json'           = $version
    'Cargo.toml'                = ([regex]::Match((Get-Content (Join-Path $repo 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"')).Groups[1].Value
    'apps/web/package.json'     = (Get-Content (Join-Path $repo 'apps\web\package.json') -Raw | ConvertFrom-Json).version
    'apps/desktop/package.json' = (Get-Content (Join-Path $repo 'apps\desktop\package.json') -Raw | ConvertFrom-Json).version
}
$wrong = $found.GetEnumerator() | Where-Object { $_.Value -ne $version }
if ($wrong) { throw "Versions differ: $(($found.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', ')" }
Write-Host "$product $version"

# 2. The build replaces target\release\rp-desktop.exe; a running copy of it locks the file.
$exe = Join-Path $repo 'target\release\rp-desktop.exe'
$running = Get-Process rp-desktop -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
if ($running) { throw "$exe is running. Close the app first." }

if (-not $OutDir) { $OutDir = Join-Path $repo "target\release-assets\$version" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# 3. Web UI first: the release EXE embeds apps/web/dist at compile time.
Push-Location (Join-Path $repo 'apps\web')
try {
    Invoke-Step 'npm ci (web)' { npm ci --no-audit --no-fund }
    Invoke-Step 'web build' { npm run build }
} finally { Pop-Location }

# 4. Release EXE + NSIS installer (bundle targets come from tauri.conf.json).
Push-Location (Join-Path $repo 'apps\desktop')
try {
    Invoke-Step 'npm ci (desktop)' { npm ci --no-audit --no-fund }
    Invoke-Step 'tauri build' { npx tauri build }
} finally { Pop-Location }

$setup = Join-Path $repo "target\release\bundle\nsis\${product}_${version}_x64-setup.exe"
if (-not (Test-Path $setup)) { throw "Installer not found: $setup" }
if (-not (Test-Path $exe)) { throw "EXE not found: $exe" }

# 5. Copy under the release names and write the checksums.
$assets = [ordered]@{
    "$product-$version-setup.exe"    = $setup
    "$product-$version-portable.exe" = $exe
}
$sums = foreach ($name in $assets.Keys) {
    $dest = Join-Path $OutDir $name
    Copy-Item $assets[$name] $dest -Force
    $hash = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name"
}
[IO.File]::WriteAllLines((Join-Path $OutDir 'SHA256SUMS.txt'), [string[]]$sums)

# 6. Show what was built: size, file properties and checksum.
foreach ($name in $assets.Keys) {
    $f = Get-Item (Join-Path $OutDir $name)
    $v = $f.VersionInfo
    '{0}  {1:N1} MB  ProductName={2}  ProductVersion={3}  FileVersion={4}' -f $name, ($f.Length / 1MB), $v.ProductName, $v.ProductVersion, $v.FileVersion
}
Get-Content (Join-Path $OutDir 'SHA256SUMS.txt')
Write-Host "Release files in $OutDir"
