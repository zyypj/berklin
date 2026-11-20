param([switch]$NoBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$out = Join-Path $root 'dist-release'

if (-not $NoBuild) {
    Push-Location (Join-Path $root 'ui')
    npm install --no-audit --no-fund
    npm run build
    Pop-Location
    Push-Location $root
    cargo build --release
    Pop-Location
    Push-Location (Join-Path $root 'relay')
    node build.mjs
    Pop-Location
    Push-Location (Join-Path $root 'android')
    .\gradlew.bat assembleRelease --console=plain
    Pop-Location
}

Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null

$stage = Join-Path $out 'windows'
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root 'target\release\berklin.exe') $stage
Copy-Item (Join-Path $root 'target\release\berklin-hook.exe') $stage
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $out "berklin-$version-windows-x64.zip")
Remove-Item -Recurse -Force $stage

$apk = Get-ChildItem (Join-Path $root 'android\app\build\outputs\apk\release') -Filter '*.apk' | Where-Object { $_.Name -notmatch 'unsigned' } | Select-Object -First 1
if (-not $apk) { throw 'APK de release assinado não encontrado' }
Copy-Item $apk.FullName (Join-Path $out "berklin-$version-android.apk")

Copy-Item (Join-Path $root 'relay\dist\server.js') (Join-Path $out "berklin-$version-relay-server.js")

Get-ChildItem $out | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    "$hash  $($_.Name)"
} | Set-Content -Encoding ascii (Join-Path $out 'SHA256SUMS.txt')

Get-ChildItem $out | Select-Object Name, Length
