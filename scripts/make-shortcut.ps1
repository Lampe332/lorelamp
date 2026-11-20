<#
.SYNOPSIS
  Creates (or replaces) the shortcut "Lorelamp.lnk" that starts the release EXE with Lorelamp's icon.

.DESCRIPTION
  Every path defaults to this repository, so nothing personal is written here:
    target\release\rp-desktop.exe           the program (build: cargo build --release -p rp-desktop)
    apps\desktop\src-tauri\icons\icon.ico   the icon (rendered by design\icon\make_icon.py)
  The icon file is used rather than the EXE's own icon: Windows keeps an EXE's old icon in its
  cache, a file named in the shortcut is read fresh.

  -RemoveOld deletes the old shortcut "RP-App.lnk" in the same folder.
  -ResetNameCache forgets the name Windows remembers for the EXE (shell MuiCache in HKCU): the
  firewall prompt and other shell dialogs then read "Lorelamp" from the EXE again instead of an old
  "rp-app". Only entries for this EXE's path are removed.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\make-shortcut.ps1 -RemoveOld -ResetNameCache
  (shortcut on the desktop)
.EXAMPLE
  powershell -File scripts\make-shortcut.ps1 -Folder $env:TEMP\lnk-test
#>
param(
    [string]$Folder = [Environment]::GetFolderPath('Desktop'),
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\rp-desktop.exe'),
    [string]$Icon = (Join-Path $PSScriptRoot '..\apps\desktop\src-tauri\icons\icon.ico'),
    [string]$Name = 'Lorelamp',
    [switch]$RemoveOld,
    [switch]$ResetNameCache
)
$ErrorActionPreference = 'Stop'

$Exe = [IO.Path]::GetFullPath($Exe)
$Icon = [IO.Path]::GetFullPath($Icon)
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "Program not found: $Exe (build it first: cargo build --release -p rp-desktop)" }
if (-not (Test-Path -LiteralPath $Icon -PathType Leaf)) { throw "Icon not found: $Icon" }
if (-not (Test-Path -LiteralPath $Folder -PathType Container)) { throw "Folder not found: $Folder" }

$path = Join-Path $Folder "$Name.lnk"
$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($path)
$lnk.TargetPath = $Exe
$lnk.WorkingDirectory = Split-Path $Exe
$lnk.IconLocation = "$Icon,0"
$lnk.Description = 'Lorelamp'
$lnk.Save()
Write-Output "shortcut: $path -> $Exe (icon $Icon)"

if ($RemoveOld) {
    $old = Join-Path $Folder 'RP-App.lnk'
    if (Test-Path -LiteralPath $old) {
        Remove-Item -LiteralPath $old
        Write-Output "removed:  $old"
    }
}

if ($ResetNameCache) {
    $key = 'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache'
    if (Test-Path $key) {
        $prefix = "$Exe."
        foreach ($p in (Get-ItemProperty $key).PSObject.Properties) {
            if ($p.Name.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                Remove-ItemProperty -Path $key -Name $p.Name
                Write-Output "name cache: removed $($p.Name)"
            }
        }
    }
}
