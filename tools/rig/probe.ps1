# probe.ps1 - is LV1's UI thread responsive? Sends WM_NULL to LV1's main window every
# 10 ms for -Seconds and reports the round trips (SendMessageTimeout, 2 s cap). Run it in
# the desktop session (the interactive task), not from SSH: window handles are per session.
#   probe.ps1 -Seconds 20 -Out $env:USERPROFILE\tunesync-probe.txt
param([int]$Seconds = 20, [string]$Out = "$env:USERPROFILE\tunesync-probe.txt")
Add-Type -Namespace TS -Name W -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr h, uint msg, System.IntPtr w, System.IntPtr l, uint flags, uint timeout, out System.IntPtr result);
'@
$p = Get-Process -Name 'eMotion LV1' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p -or $p.MainWindowHandle -eq 0) { "LV1 window not found" | Set-Content $Out; exit 1 }
$h = $p.MainWindowHandle
$sw = [Diagnostics.Stopwatch]::new()
$times = New-Object System.Collections.Generic.List[double]
$end = (Get-Date).AddSeconds($Seconds)
$slow = @()
while ((Get-Date) -lt $end) {
    $r = [IntPtr]::Zero
    $sw.Restart()
    [void][TS.W]::SendMessageTimeout($h, 0, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$r)
    $ms = $sw.Elapsed.TotalMilliseconds
    $times.Add($ms)
    if ($ms -gt 50) { $slow += ("{0:HH:mm:ss.fff} {1:N1} ms" -f (Get-Date), $ms) }
    Start-Sleep -Milliseconds 10
}
$s = $times | Sort-Object
$p99 = $s[[int]([math]::Floor($s.Count * 0.99)) - 1]
@("samples $($s.Count)  median {0:N2} ms  p99 {1:N2} ms  max {2:N2} ms" -f $s[[int]($s.Count / 2)], $p99, $s[-1]) + $slow | Set-Content $Out
Get-Content $Out
