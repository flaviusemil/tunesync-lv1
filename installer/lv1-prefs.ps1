# lv1-prefs.ps1 - add TuneSync to eMotion LV1's control surfaces, or take it out again,
# by editing LV1's preferences while LV1 is closed (LV1 rewrites the file when it closes).
#
#   lv1-prefs.ps1 -Add       add the <SurfaceDriver> entry (once; a backup is kept first)
#   lv1-prefs.ps1 -Remove    remove exactly that entry
#
# Exit codes: 0 done, 2 LV1 is running, 3 no LV1 preferences yet, 4 the file is not what
# this script knows. Adding TuneSync in LV1's Setup > Control Surfaces does the same.
param([switch]$Add, [switch]$Remove)
$ErrorActionPreference = 'Stop'

$rel    = 'Modules\TuneSync_Control.dll'
$prefs  = Join-Path $env:APPDATA 'Waves Audio\Preferences\LV1.Mixer SoundGrid.preferences'
$backup = "$prefs.before-tunesync"

if (Get-Process -Name 'eMotion LV1' -ErrorAction SilentlyContinue) { 'eMotion LV1 is running: close it first'; exit 2 }
if (-not (Test-Path $prefs)) { "no LV1 preferences at $prefs (start LV1 once, or add TuneSync in Setup > Control Surfaces)"; exit 3 }

# LV1 writes the file without a BOM, LF line ends and 4-space indent: keep it that way.
function Save-Prefs([xml]$doc) {
    $ws = New-Object System.Xml.XmlWriterSettings
    $ws.Encoding = New-Object System.Text.UTF8Encoding($false)
    $ws.Indent = $true; $ws.IndentChars = '    '; $ws.NewLineChars = "`n"
    $w = [System.Xml.XmlWriter]::Create($prefs, $ws); $doc.Save($w); $w.Close()
}

[xml]$x = Get-Content -Raw $prefs
$drivers = $x.SelectSingleNode('/Mixer/SurfaceDrivers')
if (-not $drivers) { 'LV1 preferences have no <SurfaceDrivers>: not changed'; exit 4 }
$ours = @($drivers.SelectNodes('SurfaceDriver') | Where-Object { $_.RelativePathFromApp -eq $rel })

if ($Remove) {
    if ($ours.Count -eq 0) { 'TuneSync was not in LV1 preferences'; exit 0 }
    foreach ($n in $ours) { [void]$drivers.RemoveChild($n) }
    Save-Prefs $x
    "removed TuneSync from LV1 preferences"
    exit 0
}

if ($ours.Count -gt 0) { 'TuneSync is already in LV1 preferences'; exit 0 }
if (-not (Test-Path $backup)) { Copy-Item $prefs $backup }
$e = $x.CreateElement('SurfaceDriver')
foreach ($kv in @(@('RelativePathFromApp', $rel), @('AssignIndex', '4294967295'),
                  @('IsActive', 'true'), @('ControllerType', 'TuneSync'))) {
    $c = $x.CreateElement($kv[0]); $c.InnerText = $kv[1]; [void]$e.AppendChild($c)
}
$ss = $x.CreateElement('Surfaces'); $s = $x.CreateElement('Surface')
# The same 8 bytes WMSD_GetPreset returns ("TS01"), so LV1 sees a preset it could have written.
foreach ($kv in @(@('PresetSize', '8'), @('PresetVersion', '1.0'), @('Preset', '54 53 30 31 00 00 00 00'))) {
    $c = $x.CreateElement($kv[0]); $c.InnerText = $kv[1]; [void]$s.AppendChild($c)
}
[void]$ss.AppendChild($s); [void]$e.AppendChild($ss); [void]$drivers.AppendChild($e)
Save-Prefs $x
"added TuneSync to LV1 preferences (backup: $backup)"
exit 0
