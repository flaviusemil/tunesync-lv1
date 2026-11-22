# click.ps1 - (desktop session) click LV1 at PHYSICAL screen pixels, DPI-aware, after
# minimising the interactive task's own terminal (it opens on top of LV1), then capture the
# screen. Every click is followed by a screenshot to check it landed.
#   click.ps1 -X 3420 -Y 1536 [-Double] [-NoClick] [-Out $env:USERPROFILE\tunesync\screen.png]
param([int]$X = -1, [int]$Y = -1, [switch]$Double, [switch]$NoClick, [string]$Out = "$env:USERPROFILE\tunesync\screen.png", [int]$Wait = 1200)
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class TSC {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool EnumWindows(P f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  public struct POINT { public int X, Y; }
  public static IntPtr Find(string cls, string titlePrefix) { IntPtr found = IntPtr.Zero;
    EnumWindows((h,l) => { var c = new StringBuilder(256); GetClassName(h, c, 256); var t = new StringBuilder(256); GetWindowText(h, t, 256);
      if (IsWindowVisible(h) && (cls == null || c.ToString() == cls) && t.ToString().StartsWith(titlePrefix)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found; }
}
"@
[TSC]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
$term = [TSC]::Find('CASCADIA_HOSTING_WINDOW_CLASS', '')
if ($term -ne [IntPtr]::Zero) { [void][TSC]::ShowWindow($term, 6) }   # SW_MINIMIZE
$lv1 = [TSC]::Find($null, 'eMotion LV1')
if (-not $NoClick -and $X -ge 0) {
  $p = New-Object TSC+POINT; $p.X = $X; $p.Y = $Y
  $top = [TSC]::GetAncestor([TSC]::WindowFromPoint($p), 2)
  $t = New-Object Text.StringBuilder 256; [void][TSC]::GetWindowText($top, $t, 256)
  # LV1 (Qt) has more than one top-level window with its title: judge by the title.
  if (-not $t.ToString().StartsWith('eMotion LV1') -and -not $t.ToString().StartsWith('TuneSync')) { "NOT clicking: the window at $X,$Y is [$t], not LV1"; exit 1 }
  [void][TSC]::SetForegroundWindow($top); Start-Sleep -Milliseconds 300
  [void][TSC]::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 150
  $n = if ($Double) { 2 } else { 1 }
  for ($i = 0; $i -lt $n; $i++) { [TSC]::mouse_event(2,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 60; [TSC]::mouse_event(4,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 90 }
  "clicked $X,$Y on [$t]"
  Start-Sleep -Milliseconds $Wait
}
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
$g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size); $bmp.Save($Out); "saved"
