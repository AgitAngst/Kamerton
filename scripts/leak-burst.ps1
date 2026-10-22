# Проверка утечки: подряд запускает короткие процессы, каждый открывает звук (тишину) и выходит.
# Kamerton в это время работает в `--watch`; память снимать до и после (см. docs/dev/development.md).
# Грузит ПК — запускать только с согласия владельца.
param(
    [int]$n = 60,
    [int]$pause = 1000,
    [string]$dir = 'E:\dev_personal\kamerton-scratch\leak'
)
New-Item -ItemType Directory -Force $dir | Out-Null
$wav = Join-Path $dir 'silence.wav'
if (-not (Test-Path $wav)) {
    # 0,1 с тишины: PCM 8 кГц, 16 бит, моно.
    $data = New-Object byte[] 1600
    $ms = New-Object IO.MemoryStream
    $w = New-Object IO.BinaryWriter($ms)
    $w.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $w.Write([int](36 + $data.Length))
    $w.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt ')); $w.Write([int]16); $w.Write([int16]1); $w.Write([int16]1)
    $w.Write([int]8000); $w.Write([int]16000); $w.Write([int16]2); $w.Write([int16]16)
    $w.Write([Text.Encoding]::ASCII.GetBytes('data')); $w.Write([int]$data.Length); $w.Write($data)
    [IO.File]::WriteAllBytes($wav, $ms.ToArray())
}
for ($i = 0; $i -lt $n; $i++) {
    Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile', '-Command', "(New-Object Media.SoundPlayer '$wav').PlaySync()" -WindowStyle Hidden -Wait
    Start-Sleep -Milliseconds $pause
}
