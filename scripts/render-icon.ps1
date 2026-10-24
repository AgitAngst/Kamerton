<#
.SYNOPSIS
  Renders the Kamerton icon files from the SVG sources in assets/icon: PNGs, kamerton.ico and raw RGBA samples.
.DESCRIPTION
  Sources: kamerton.svg (the full mark, 64 px and up) and kamerton-small.svg (wider pale tracks and bigger knobs
  without the gap rings, up to 48 px). kamerton-{16,32,64}.rgba are raw RGBA samples of the same sizes (rendered
  8 times larger and averaged): the tests of the frame rasterizer (src/mark_render.rs) compare against them.
  Needs resvg on PATH
  (scoop install resvg). Running it again without changes to the SVGs gives the same files.
#>
[CmdletBinding()]
param([string]$Dir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'assets\icon'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

function Render([string]$svg, [int]$size, [string]$png) {
    & resvg -w $size -h $size (Join-Path $Dir $svg) (Join-Path $Dir $png)
    if ($LASTEXITCODE -ne 0) { throw "resvg failed for $svg at $size px" }
}

# Raw RGBA sample (straight alpha, rows top to bottom): the SVG rendered 8 times larger and averaged in 8 x 8
# blocks with premultiplied alpha. That is the true coverage; resvg's own anti-aliasing at 16-32 px is coarser.
function Rgba([string]$svg, [int]$size, [string]$out) {
    $f = 8
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) "kamerton-$size-x$f.png"
    & resvg -w ($size * $f) -h ($size * $f) (Join-Path $Dir $svg) $tmp
    if ($LASTEXITCODE -ne 0) { throw "resvg failed for $svg at $($size * $f) px" }
    $bmp = New-Object System.Drawing.Bitmap $tmp
    try {
        $rect = New-Object System.Drawing.Rectangle 0, 0, $bmp.Width, $bmp.Height
        $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $bytes = New-Object byte[] ($data.Stride * $bmp.Height)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $bmp.UnlockBits($data)
        $rgba = New-Object byte[] ($size * $size * 4)
        for ($y = 0; $y -lt $size; $y++) {
            for ($x = 0; $x -lt $size; $x++) {
                $r = 0.0; $g = 0.0; $b = 0.0; $a = 0.0
                for ($sy = 0; $sy -lt $f; $sy++) {
                    $row = ($y * $f + $sy) * $data.Stride
                    for ($sx = 0; $sx -lt $f; $sx++) {
                        $s = $row + ($x * $f + $sx) * 4
                        $pa = $bytes[$s + 3] / 255.0
                        $b += $bytes[$s] * $pa; $g += $bytes[$s + 1] * $pa; $r += $bytes[$s + 2] * $pa; $a += $pa
                    }
                }
                $d = ($y * $size + $x) * 4
                if ($a -gt 0) {
                    $rgba[$d] = [byte][math]::Round($r / $a); $rgba[$d + 1] = [byte][math]::Round($g / $a); $rgba[$d + 2] = [byte][math]::Round($b / $a)
                }
                $rgba[$d + 3] = [byte][math]::Round($a / ($f * $f) * 255)
            }
        }
        [System.IO.File]::WriteAllBytes((Join-Path $Dir $out), $rgba)
    } finally { $bmp.Dispose(); Remove-Item $tmp -ErrorAction SilentlyContinue }
}

foreach ($s in 16, 24, 32, 48) { Render 'kamerton-small.svg' $s "kamerton-$s.png" }
foreach ($s in 64, 128, 256, 512) { Render 'kamerton.svg' $s "kamerton-$s.png" }
foreach ($s in 16, 32) { Rgba 'kamerton-small.svg' $s "kamerton-$s.rgba" }
Rgba 'kamerton.svg' 64 'kamerton-64.rgba'

# kamerton.ico: PNG frames (Windows Vista and later), 16 to 256.
$sizes = 16, 24, 32, 48, 64, 128, 256
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($ms)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
$blobs = @()
foreach ($s in $sizes) {
    $b = [System.IO.File]::ReadAllBytes((Join-Path $Dir "kamerton-$s.png"))
    $blobs += , $b
    # Width and height bytes: 0 means 256.
    $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$b.Length); $w.Write([uint32]$offset)
    $offset += $b.Length
}
foreach ($b in $blobs) { $w.Write($b) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $Dir 'kamerton.ico'), $ms.ToArray())
Write-Host "Icon files written to $Dir"
