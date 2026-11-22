# log.ps1 - the newest lines of TuneSync's log on this host.
param([int]$Lines = 40)
$f = 'C:\ProgramData\TuneSync\tunesync.log'
if (Test-Path $f) { Get-Content $f -Tail $Lines } else { "no $f yet" }
