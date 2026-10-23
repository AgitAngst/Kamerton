# Переносная сборка на ПК по соглашению выпусков Anvil: target\package\kamerton-X.Y.Z-windows-x64.zip и
# SHA256SUMS — то же, что собирает workflow выпуска по тегу. Ничего не публикует.
# Запуск: powershell -File scripts\package.ps1 [-NoBuild]
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $NoBuild) {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
$exe = Join-Path $root 'target\release\kamerton.exe'
if (-not (Test-Path $exe)) { throw "нет $exe" }
$out = Join-Path $root 'target\package'
New-Item -ItemType Directory -Force $out | Out-Null
$name = "kamerton-$version-windows-x64.zip"
$zip = Join-Path $out $name
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path $exe -DestinationPath $zip
$sum = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText((Join-Path $out 'SHA256SUMS'), "$sum  $name`n")
Write-Output "$zip"
Write-Output "$sum  $name"
