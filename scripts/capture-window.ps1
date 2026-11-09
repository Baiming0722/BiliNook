param([int]$ProcessId, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class BiliCapture {
    public delegate bool Callback(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback callback, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
}
'@
[BiliCapture]::SetThreadDpiAwarenessContext([IntPtr]::new(-4)) | Out-Null
$handles = [System.Collections.Generic.List[IntPtr]]::new()
[BiliCapture]::EnumWindows({param($h,$l)
    [uint32]$windowPid = 0
    [BiliCapture]::GetWindowThreadProcessId($h,[ref]$windowPid) | Out-Null
    if ($windowPid -eq $ProcessId) {
        $title = [System.Text.StringBuilder]::new(512)
        [BiliCapture]::GetWindowText($h,$title,512) | Out-Null
        $rect = [BiliCapture+Rect]::new()
        [BiliCapture]::GetWindowRect($h,[ref]$rect) | Out-Null
        if ($title.ToString() -eq 'BiliNook') { $handles.Add($h) }
    }
    return $true
},[IntPtr]::Zero) | Out-Null
if ($handles.Count -ne 1) { throw "Expected one application window, found $($handles.Count)" }
$windowHandle = $handles[0]
$wasVisible = [BiliCapture]::IsWindowVisible($windowHandle)
[BiliCapture]::ShowWindow($windowHandle,4) | Out-Null
Start-Sleep -Milliseconds 500
$rect = [BiliCapture+Rect]::new()
[BiliCapture]::GetWindowRect($windowHandle,[ref]$rect) | Out-Null
$bitmap = [System.Drawing.Bitmap]::new($rect.Right-$rect.Left,$rect.Bottom-$rect.Top)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$dc = $graphics.GetHdc()
try {
    if (![BiliCapture]::PrintWindow($windowHandle,$dc,2)) { throw 'PrintWindow failed' }
} finally {
    $graphics.ReleaseHdc($dc)
    $graphics.Dispose()
    if (!$wasVisible) { [BiliCapture]::ShowWindow($windowHandle,0) | Out-Null }
}
try { $bitmap.Save($OutputPath,[System.Drawing.Imaging.ImageFormat]::Png) }
finally { $bitmap.Dispose() }
Write-Output $OutputPath
