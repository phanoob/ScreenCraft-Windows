# Build a release ScreenCraft.exe for Windows (the UI is embedded in the binary).
#   .\scripts\build-windows.ps1            -> dist\ScreenCraft.exe
#   .\scripts\build-windows.ps1 -Install   -> also copies it to %LOCALAPPDATA%\Programs\ScreenCraft
#                                             and adds a Start menu shortcut
param([switch]$Install)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --release -p screencraft
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    New-Item -ItemType Directory -Force dist | Out-Null
    Copy-Item target\release\screencraft.exe dist\ScreenCraft.exe -Force
    if ($Install) {
        $dir = Join-Path $env:LOCALAPPDATA "Programs\ScreenCraft"
        New-Item -ItemType Directory -Force $dir | Out-Null
        Copy-Item dist\ScreenCraft.exe $dir -Force
        $lnk = Join-Path ([Environment]::GetFolderPath("Programs")) "ScreenCraft.lnk"
        $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
        $shortcut.TargetPath = Join-Path $dir "ScreenCraft.exe"
        $shortcut.Save()
        Write-Host "Installed to $dir (Start menu: ScreenCraft)"
    } else {
        Write-Host "Built dist\ScreenCraft.exe"
    }
} finally {
    Pop-Location
}
