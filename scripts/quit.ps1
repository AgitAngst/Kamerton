# Закрыть запущенный Kamerton как «Выход» в меню трея: значок убирается, программа выходит.
Add-Type -Name U -Namespace W -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
'@
$h = [W.U]::FindWindow('KamertonTray', [NullString]::Value)
if ($h -eq [IntPtr]::Zero) { Write-Output 'not running'; exit 0 }
[W.U]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Write-Output 'closed'
