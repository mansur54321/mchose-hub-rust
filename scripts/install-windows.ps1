param(
    [string]$SourceDirectory = $PSScriptRoot,
    [ValidateSet('ru', 'en')][string]$Language,
    [switch]$DesktopShortcut
)
$ErrorActionPreference = 'Stop'
$binary = Join-Path $SourceDirectory 'mchose-hub-rust.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "mchose-hub-rust.exe was not found in $SourceDirectory. Extract the complete Windows package first."
}
if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is unavailable.' }
$destination = Join-Path $env:LOCALAPPDATA 'Programs\MCHOSE Hub Rust'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
$installed = Join-Path $destination 'mchose-hub-rust.exe'
if (Test-Path -LiteralPath $installed) {
    $backup = Join-Path $destination ('previous-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fffffff') + '.exe')
    Copy-Item -LiteralPath $installed -Destination $backup
}
Copy-Item -LiteralPath $binary -Destination $installed -Force
$icon = Join-Path $SourceDirectory 'mchose-icon.ico'
if (Test-Path -LiteralPath $icon) {
    Copy-Item -LiteralPath $icon -Destination (Join-Path $destination 'mchose-icon.ico') -Force
}
$shell = New-Object -ComObject WScript.Shell
$locations = @([Environment]::GetFolderPath('Programs'))
if ($DesktopShortcut) { $locations += [Environment]::GetFolderPath('Desktop') }
foreach ($location in $locations) {
    $shortcut = $shell.CreateShortcut((Join-Path $location 'MCHOSE Hub Rust.lnk'))
    $shortcut.TargetPath = $installed
    $shortcut.WorkingDirectory = $destination
    $shortcut.Description = 'Control MCHOSE A7 V3 Ultra+, MagDock and Ace75'
    $shortcut.IconLocation = "$installed,0"
    if ($Language) { $shortcut.Arguments = "--lang $Language" }
    $shortcut.Save()
}
Write-Output "Installed: $installed"
Write-Output 'Settings and backups are kept separately in LOCALAPPDATA\mchose-native.'
