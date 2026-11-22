#!/bin/sh
# install.sh - install TuneSync for eMotion LV1 on this Mac, or take it out again.
#
#   sudo ./install.sh                 copy TuneSync_Control.bundle into LV1's Modules folder,
#                                     make /Library/Application Support/TuneSync (settings and
#                                     log, writable by every local user), and add TuneSync to
#                                     LV1's control surfaces (LV1's preferences, backed up first)
#   sudo ./install.sh --no-surface    the same, without touching LV1's preferences (then add
#                                     TuneSync in LV1's Setup > Control Surfaces)
#   sudo ./install.sh --remove        take the bundle and the surface entry out again
#                                     (settings.ini and tunesync.log are kept)
#
# Without sudo it still works when LV1's Modules folder is yours: TuneSync then keeps its
# settings and log in ~/Library/Application Support/TuneSync.
# LV1 must be closed: it holds the bundle open and rewrites its preferences when it quits.
# Set LV1_DIR when LV1 is not in /Applications/Waves/eMotion LV1.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
LV1_DIR=${LV1_DIR:-"/Applications/Waves/eMotion LV1"}
MODULES="$LV1_DIR/Modules"
BUNDLE=TuneSync_Control.bundle
DATA="/Library/Application Support/TuneSync"
mode=install; surface=1
for a in "$@"; do
    case $a in
        --remove) mode=remove ;;
        --no-surface) surface=0 ;;
        *) sed -n '2,17s/^# \{0,1\}//p' "$0"; exit 1 ;;
    esac
done

if pgrep -x "eMotion LV1" >/dev/null 2>&1; then echo "eMotion LV1 is running: quit it first"; exit 2; fi
[ -d "$MODULES" ] || { echo "no eMotion LV1 at $LV1_DIR (set LV1_DIR=... if it is elsewhere)"; exit 3; }
root=0; [ "$(id -u)" = 0 ] && root=1
if [ $root = 0 ] && [ ! -w "$MODULES" ]; then
    echo "LV1's Modules folder is not writable for $(id -un): run it with sudo"; exit 1
fi

if [ $mode = remove ]; then
    rm -rf "${MODULES:?}/$BUNDLE"
    echo "removed $MODULES/$BUNDLE"
    "$HERE/lv1-prefs.sh" --remove || true
    echo "settings and log are kept in $DATA (or ~/Library/Application Support/TuneSync)"
    exit 0
fi

[ -d "$HERE/$BUNDLE" ] || { echo "no $BUNDLE next to install.sh"; exit 1; }
rm -rf "${MODULES:?}/$BUNDLE"
cp -R "$HERE/$BUNDLE" "$MODULES/"
# A bundle unpacked from a download carries the quarantine flag, and macOS would refuse
# to load it into LV1.
xattr -dr com.apple.quarantine "$MODULES/$BUNDLE" 2>/dev/null || true
chmod -R go-w "$MODULES/$BUNDLE"
echo "installed $MODULES/$BUNDLE"

if [ $root = 1 ]; then
    mkdir -p "$DATA"
    chgrp staff "$DATA" && chmod 2775 "$DATA"   # LV1 runs as the user: every local user may write
    echo "settings and log: $DATA"
else
    echo "settings and log: ~/Library/Application Support/TuneSync (no sudo, so not $DATA)"
fi

if [ $surface = 1 ]; then
    "$HERE/lv1-prefs.sh" --add || echo "LV1's control surfaces were not changed: add TuneSync in LV1's Setup > Control Surfaces"
fi
echo "done: start eMotion LV1; TuneSync's settings open from Setup > Control Surfaces"
