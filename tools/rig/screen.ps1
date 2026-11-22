# screen.ps1 - (desktop session) the whole screen in real pixels, plus the visible top-level
# windows from the top of the z-order down, with their rects and DPI.
#   screen.ps1 -Out C:\Users\you\tunesync\screen.png
param([string]$Out = "C:\Users\you\tunesync\screen.png")
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class TSS {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern IntPtr GetTopWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  public struct RECT { public int L, T, R, B; }
}
"@
[TSS]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
"screen $($b.Width)x$($b.Height) at $($b.X),$($b.Y); foreground " + [TSS]::GetForegroundWindow()
$h = [TSS]::GetTopWindow([IntPtr]::Zero); $n = 0
while ($h -ne [IntPtr]::Zero -and $n -lt 400) {
  if ([TSS]::IsWindowVisible($h)) {
    $t = New-Object Text.StringBuilder 256; [void][TSS]::GetWindowText($h, $t, 256)
    $c = New-Object Text.StringBuilder 256; [void][TSS]::GetClassName($h, $c, 256)
    $r = New-Object TSS+RECT; [void][TSS]::GetWindowRect($h, [ref]$r)
    if (($r.R - $r.L) -gt 50 -and ($r.B - $r.T) -gt 50) { "  $h [$t] <$c> $($r.L),$($r.T) $($r.R-$r.L)x$($r.B-$r.T) dpi $([TSS]::GetDpiForWindow($h))" }
  }
  $h = [TSS]::GetWindow($h, 2); $n++
}
$bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
$g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size); $bmp.Save($Out); "saved $Out"
