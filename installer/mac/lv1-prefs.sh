#!/bin/sh
# lv1-prefs.sh - add TuneSync to eMotion LV1's control surfaces on this Mac, or take it out
# again, by editing LV1's preferences while LV1 is closed (LV1 rewrites the file when it
# quits, so an edit made while it runs is lost). The macOS twin of installer/lv1-prefs.ps1.
#
#   lv1-prefs.sh --add [--prefs FILE]      add the <SurfaceDriver> entry (once; a backup
#                                          is kept first, FILE.before-tunesync)
#   lv1-prefs.sh --remove [--prefs FILE]   remove exactly that entry
#
# Without --prefs it edits the LV1 preferences of the user running it (or of $SUDO_USER
# under sudo), found under ~/Library/Preferences.
# Exit codes: 0 done, 2 LV1 is running, 3 no LV1 preferences yet, 4 the file is not what
# this script knows. Adding TuneSync in LV1's Setup > Control Surfaces does the same.
set -eu
BUNDLE=TuneSync_Control.bundle
NAME='LV1.Mixer SoundGrid.preferences'
mode=; prefs=
while [ $# -gt 0 ]; do
    case $1 in
        --add) mode=add ;;
        --remove) mode=remove ;;
        --prefs) prefs=${2:-}; shift ;;
        *) echo "usage: lv1-prefs.sh --add|--remove [--prefs FILE]"; exit 1 ;;
    esac
    shift
done
[ -n "$mode" ] || { echo "usage: lv1-prefs.sh --add|--remove [--prefs FILE]"; exit 1; }

if [ -z "${TUNESYNC_TEST:-}" ] && pgrep -x "eMotion LV1" >/dev/null 2>&1; then
    echo "eMotion LV1 is running: quit it first"; exit 2
fi

if [ -z "$prefs" ]; then
    if [ -n "${SUDO_USER:-}" ]; then home=$(eval echo "~$SUDO_USER"); else home=$HOME; fi
    for d in "Waves Preferences" "Waves Audio/Preferences" "Waves Audio"; do
        [ -f "$home/Library/Preferences/$d/$NAME" ] && { prefs="$home/Library/Preferences/$d/$NAME"; break; }
    done
    if [ -z "$prefs" ]; then
        prefs=$(find "$home/Library/Preferences" -maxdepth 3 -name "$NAME" 2>/dev/null | head -1)
    fi
    [ -n "$prefs" ] || { echo "no LV1 preferences under $home/Library/Preferences (start LV1 once, or add TuneSync in Setup > Control Surfaces)"; exit 3; }
fi
[ -f "$prefs" ] || { echo "no LV1 preferences at $prefs"; exit 3; }
if ! grep -q '<SurfaceDrivers>' "$prefs" || ! grep -qE '^[[:space:]]*</SurfaceDrivers>[[:space:]]*$' "$prefs"; then
    echo "LV1 preferences have no <SurfaceDrivers> list as LV1 writes it: not changed ($prefs)"; exit 4
fi

# Ours is the entry whose path ends in our bundle (LV1 writes it the same way when the
# surface is added in Setup).
has_ours() { grep -q "<RelativePathFromApp>[^<]*$BUNDLE</RelativePathFromApp>" "$prefs"; }
tmp="$prefs.tunesync-tmp"
# Written over the file in place (cat >), so its owner and mode stay LV1's.
commit() { cat "$tmp" > "$prefs"; rm -f "$tmp"; }

if [ "$mode" = remove ]; then
    has_ours || { echo "TuneSync was not in LV1 preferences"; exit 0; }
    awk -v b="$BUNDLE" '
        function flush() { if (!(buf ~ ("<RelativePathFromApp>[^<]*" b "</RelativePathFromApp>"))) printf "%s", buf; buf = ""; inside = 0 }
        inside == 0 && /<SurfaceDriver>/ { inside = 1 }
        inside { buf = buf $0 "\n"; if (/<\/SurfaceDriver>/) flush(); next }
        { print }
        END { if (inside) printf "%s", buf }
    ' "$prefs" > "$tmp"
    commit
    echo "removed TuneSync from LV1 preferences ($prefs)"
    exit 0
fi

if has_ours; then echo "TuneSync is already in LV1 preferences"; exit 0; fi
# The same folder style LV1 uses for its own GUI driver's entry (Modules/SurfaceDriver_App.bundle).
own=$(sed -n 's:.*<RelativePathFromApp>\([^<]*\)SurfaceDriver_App\.bundle</RelativePathFromApp>.*:\1:p' "$prefs" | head -1)
rel="${own:-Modules/}$BUNDLE"
backup="$prefs.before-tunesync"
[ -f "$backup" ] || { cp -p "$prefs" "$backup"; [ -n "${SUDO_USER:-}" ] && chown "$SUDO_USER" "$backup" 2>/dev/null || true; }
awk -v rel="$rel" '
    /<\/SurfaceDrivers>/ && !done {
        ind = $0; sub(/<.*/, "", ind); i = ind "    "
        print i "<SurfaceDriver>"
        print i "    <RelativePathFromApp>" rel "</RelativePathFromApp>"
        print i "    <AssignIndex>4294967295</AssignIndex>"
        print i "    <IsActive>true</IsActive>"
        print i "    <ControllerType>TuneSync</ControllerType>"
        print i "    <Surfaces>"
        print i "        <Surface>"
        print i "            <PresetSize>8</PresetSize>"
        print i "            <PresetVersion>1.0</PresetVersion>"
        print i "            <Preset>54 53 30 31 00 00 00 00</Preset>"
        print i "        </Surface>"
        print i "    </Surfaces>"
        print i "</SurfaceDriver>"
        done = 1
    }
    { print }
' "$prefs" > "$tmp"
commit
echo "added TuneSync to LV1 preferences as $rel ($prefs; backup: $backup)"
exit 0
