param([switch]$NoBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (-not $NoBuild) {
    Push-Location (Join-Path $root 'ui')
    npm install --no-audit --no-fund
    npm run build
    Pop-Location
    Push-Location $root
    cargo build --release
    Pop-Location
}

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Berklin'
Get-Process berklin -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $root 'target\release\berklin.exe') $dest -Force
Copy-Item (Join-Path $root 'target\release\berklin-hook.exe') $dest -Force

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Berklin.lnk'))
$link.TargetPath = Join-Path $dest 'berklin.exe'
$link.WorkingDirectory = $dest
$link.Save()

Start-Process (Join-Path $dest 'berklin.exe')
Write-Host "Berklin instalado em $dest"
