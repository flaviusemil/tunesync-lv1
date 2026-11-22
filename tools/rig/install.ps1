# install.ps1 - install TuneSync on an LV1 host from SSH, LV1 closed, and say what changed.
#   install.ps1 -Setup <path\TuneSync-x.y.z-setup.exe> [-NoSurface]
#   install.ps1 -Uninstall
param([string]$Setup, [switch]$NoSurface, [switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$modules = 'C:\Program Files (x86)\Waves\eMotion LV1\Modules'
$dll = Join-Path $modules 'TuneSync_Control.dll'
$prefs = Join-Path $env:APPDATA 'Waves Audio\Preferences\LV1.Mixer SoundGrid.preferences'
if (Get-Process -Name 'eMotion LV1' -ErrorAction SilentlyContinue) { 'eMotion LV1 is running: close it first'; exit 2 }
if ($Uninstall) {
    $u = Get-ChildItem 'C:\Program Files\TuneSync\unins*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $u) { 'TuneSync is not installed'; exit 0 }
    Start-Process -Wait -FilePath $u.FullName -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES'
} else {
    if (-not (Test-Path $Setup)) { "no setup at $Setup"; exit 1 }
    $tasks = if ($NoSurface) { '' } else { 'addsurface' }
    $log = Join-Path $env:TEMP 'tunesync-setup.log'
    Start-Process -Wait -FilePath $Setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', "/TASKS=$tasks", "/LOG=$log"
    "setup log: $log"
}
"DLL: " + $(if (Test-Path $dll) { (Get-Item $dll).Length.ToString() + ' bytes, ' + (Get-Item $dll).VersionInfo.FileVersion } else { 'absent' })
"ProgramData\TuneSync: " + (Test-Path 'C:\ProgramData\TuneSync') + $(if (Test-Path 'C:\ProgramData\TuneSync') { ' acl: ' + ((Get-Acl 'C:\ProgramData\TuneSync').Access | Where-Object { $_.IdentityReference -match 'Users' } | ForEach-Object { "$($_.IdentityReference)=$($_.FileSystemRights)" }) -join '; ' })
if (Test-Path $prefs) {
    [xml]$x = Get-Content -Raw $prefs
    "prefs surfaces: " + (($x.SelectNodes('/Mixer/SurfaceDrivers/SurfaceDriver') | ForEach-Object { $_.ControllerType }) -join ', ')
}
