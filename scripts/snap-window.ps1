# Снимок окна (PrintWindow: само окно, экран не читается — работает и на заблокированном) в PNG.
# Окно — по HWND или по заголовку. Курсор не трогает.
param(
    [string]$out = 'E:\dev_personal\kamerton-scratch\snap.png',
    [string]$title = 'Kamerton',
    [int]$margin = 0,
    [long]$hwnd = 0
)
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class W {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
'@
[W]::SetProcessDPIAware() | Out-Null
$h = if ($hwnd -ne 0) { [IntPtr]$hwnd } else { [W]::FindWindow($null, $title) }
if ($h -eq [IntPtr]::Zero -or -not [W]::IsWindowVisible($h)) { Write-Output 'window not visible'; exit 1 }
$r = New-Object W+RECT
[W]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hh = $r.B - $r.T
$bmp = New-Object Drawing.Bitmap ($w + 2 * $margin), ($hh + 2 * $margin)
$g = [Drawing.Graphics]::FromImage($bmp)
$g.Clear([Drawing.Color]::Gray)
$win = New-Object Drawing.Bitmap $w, $hh
$wg = [Drawing.Graphics]::FromImage($win)
$dc = $wg.GetHdc()
[W]::PrintWindow($h, $dc, 2) | Out-Null   # PW_RENDERFULLCONTENT
$wg.ReleaseHdc($dc)
$g.DrawImage($win, $margin, $margin)
$bmp.Save($out, [Drawing.Imaging.ImageFormat]::Png)
Write-Output ("{0}x{1} at {2},{3} -> {4}" -f $w, $hh, $r.L, $r.T, $out)
