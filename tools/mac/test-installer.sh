#!/bin/bash
# test-installer.sh - the Mac installer scripts on any host: installer/mac/lv1-prefs.sh
# against LV1-style preferences (add, add again, remove, the refusals), then install.sh
# without sudo into a fake LV1 folder and out again. Exit 0 = every check passed.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
P="$ROOT/installer/mac/lv1-prefs.sh"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export TUNESYNC_TEST=1
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }

cat > "$T/orig" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<Mixer>
    <Version>17</Version>
    <SurfaceDrivers>
        <SurfaceDriver>
            <RelativePathFromApp>Modules/SurfaceDriver_App.bundle</RelativePathFromApp>
            <AssignIndex>4294967295</AssignIndex>
            <IsActive>true</IsActive>
            <ControllerType>LV1</ControllerType>
            <Surfaces><Surface><PresetSize>97</PresetSize><PresetVersion>2.0</PresetVersion><Preset>00 11</Preset></Surface></Surfaces>
        </SurfaceDriver>
    </SurfaceDrivers>
</Mixer>
EOF
F="$T/LV1.Mixer SoundGrid.preferences"
cp "$T/orig" "$F"

out=$(sh "$P" --add --prefs "$F")
check "add says so" '[[ $out == "added TuneSync to LV1 preferences as Modules/TuneSync_Control.bundle"* ]]'
check "one entry" '[ "$(grep -c "<RelativePathFromApp>Modules/TuneSync_Control.bundle</RelativePathFromApp>" "$F")" = 1 ]'
check "inside SurfaceDrivers, after LV1's own" 'awk "/SurfaceDriver_App/{a=NR} /TuneSync_Control/{b=NR} /<\/SurfaceDrivers>/{c=NR} END{exit !(a<b && b<c)}" "$F"'
check "the preset LV1 would write" 'grep -q "<Preset>54 53 30 31 00 00 00 00</Preset>" "$F"'
check "indented like LV1" 'grep -q "^        <SurfaceDriver>$" "$F" && grep -q "^            <ControllerType>TuneSync</ControllerType>$" "$F"'
check "backup is the original" 'cmp -s "$T/orig" "$F.before-tunesync"'
if command -v xmllint >/dev/null; then check "still well-formed XML" 'xmllint --noout "$F"'; fi
if command -v python3 >/dev/null; then check "still well-formed XML (python)" 'python3 -I -c "import sys,xml.dom.minidom as m; m.parse(sys.argv[1])" "$F"'; fi

out=$(sh "$P" --add --prefs "$F")
check "add twice: once" '[ "$out" = "TuneSync is already in LV1 preferences" ] && [ "$(grep -c TuneSync_Control "$F")" = 1 ]'

out=$(sh "$P" --remove --prefs "$F")
check "remove gives the original back" 'cmp -s "$T/orig" "$F"'
out=$(sh "$P" --remove --prefs "$F")
check "remove again: nothing to do" '[ "$out" = "TuneSync was not in LV1 preferences" ]'

# an entry LV1 itself wrote (Setup > Control Surfaces) counts as ours
sed 's#</SurfaceDrivers>#        <SurfaceDriver><RelativePathFromApp>Modules/TuneSync_Control.bundle</RelativePathFromApp><ControllerType>TuneSync</ControllerType></SurfaceDriver>\n    </SurfaceDrivers>#' "$T/orig" > "$F"
out=$(sh "$P" --add --prefs "$F")
check "LV1's own entry is recognised" '[ "$out" = "TuneSync is already in LV1 preferences" ]'
sh "$P" --remove --prefs "$F" >/dev/null
check "and removed (one-line entry)" 'cmp -s "$T/orig" "$F"'

# refusals
printf '<Mixer><SurfaceDrivers/></Mixer>\n' > "$F"
set +e; sh "$P" --add --prefs "$F" >/dev/null; rc=$?; set -e
check "unknown layout: exit 4, unchanged" '[ $rc = 4 ] && [ "$(cat "$F")" = "<Mixer><SurfaceDrivers/></Mixer>" ]'
set +e; sh "$P" --add --prefs "$T/missing" >/dev/null; rc=$?; set -e
check "no preferences: exit 3" '[ $rc = 3 ]'

# found under a home folder, as LV1 keeps it
mkdir -p "$T/home/Library/Preferences/Waves Preferences"
cp "$T/orig" "$T/home/Library/Preferences/Waves Preferences/LV1.Mixer SoundGrid.preferences"
out=$(env -u SUDO_USER HOME="$T/home" sh "$P" --add)
check "finds ~/Library/Preferences/Waves Preferences" '[[ $out == *"Waves Preferences/LV1.Mixer SoundGrid.preferences"* ]]'

# install.sh without sudo, into a fake LV1 whose Modules folder is ours
mkdir -p "$T/lv1/Modules" "$T/zip/TuneSync_Control.bundle/Contents/MacOS"
echo binary > "$T/zip/TuneSync_Control.bundle/Contents/MacOS/TuneSync_Control"
cp "$ROOT/installer/mac/install.sh" "$ROOT/installer/mac/lv1-prefs.sh" "$T/zip/"
cp "$T/orig" "$T/home/Library/Preferences/Waves Preferences/LV1.Mixer SoundGrid.preferences"
out=$(env -u SUDO_USER HOME="$T/home" LV1_DIR="$T/lv1" sh "$T/zip/install.sh" 2>&1)
check "install.sh: the bundle is in Modules" '[ -f "$T/lv1/Modules/TuneSync_Control.bundle/Contents/MacOS/TuneSync_Control" ]'
check "install.sh: the surface is added" 'grep -q "TuneSync_Control.bundle" "$T/home/Library/Preferences/Waves Preferences/LV1.Mixer SoundGrid.preferences"'
check "install.sh: says where the settings go without sudo" '[[ $out == *"~/Library/Application Support/TuneSync"* ]]'
out=$(env -u SUDO_USER HOME="$T/home" LV1_DIR="$T/lv1" sh "$T/zip/install.sh" --remove 2>&1)
check "install.sh --remove: bundle gone" '[ ! -e "$T/lv1/Modules/TuneSync_Control.bundle" ]'
check "install.sh --remove: preferences as before" 'cmp -s "$T/orig" "$T/home/Library/Preferences/Waves Preferences/LV1.Mixer SoundGrid.preferences"'
set +e; out=$(LV1_DIR="$T/nowhere" sh "$T/zip/install.sh" 2>&1); rc=$?; set -e
check "install.sh: no LV1 there: exit 3" '[ $rc = 3 ]'

[ $fail = 0 ] && echo "all checks passed" || { echo "SOME CHECKS FAILED"; exit 1; }
