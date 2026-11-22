#!/bin/bash
# rig-test.sh <step> - TuneSync in the real eMotion LV1 on a Mac, one step at a time over
# SSH. Installing and starting or quitting LV1 change someone's machine: run those steps
# only when its owner said so.
#
#   MAC=user@host [MAC_JUMP=user@jump:port] tools/mac/rig-test.sh <step>
#
#   check          read only: macOS and CPU, LV1's version, whether LV1 runs, whether its
#                  Modules folder is writable, where its preferences are, TuneSync's files
#   install        (LV1 closed) dist/mac/TuneSync-<v>-macos.zip to the Mac, unzipped in
#                  ~/tunesync-install, then its install.sh (without sudo when the Modules
#                  folder is the user's; otherwise it prints the sudo line for a person);
#                  seeds settings.ini with RTP-MIDI on when there is none yet
#   cue <cue>...   rtpsend on the Mac to TuneSync's session on 127.0.0.1:5004, e.g.
#                  `cue cc 20 4 key D`; then the last lines of tunesync.log
#   log            the last 40 lines of tunesync.log
#   remove         (LV1 closed) install.sh --remove
#
# The steps a person does in LV1 (they need the screen): start LV1 (and answer its "last
# session" dialog), put Tune RT on a few channels (one on Lock), open Setup > Control
# Surfaces, select TuneSync and its settings button: the window must open.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
M=${MAC:?set MAC=user@host of the Mac}
SSH=(ssh -o BatchMode=yes -o ConnectTimeout=20)
[ -n "${MAC_JUMP:-}" ] && SSH+=(-J "$MAC_JUMP")
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
step=${1:-}; shift || true
remote() { "${SSH[@]}" "$M" bash -s -- "$@"; }

# Shared on the Mac: where TuneSync keeps its files (as sys::data_dir picks them).
FIND_DATA='D="/Library/Application Support/TuneSync"; [ -w "$D" ] || D="$HOME/Library/Application Support/TuneSync"'

case $step in
check)
    remote <<REMOTE
echo "macOS \$(sw_vers -productVersion) (\$(sw_vers -buildVersion)) on \$(uname -m)"
L="/Applications/Waves/eMotion LV1"
if [ -d "\$L/eMotion LV1.app" ]; then
    echo "LV1 \$(defaults read "\$L/eMotion LV1.app/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo '?')"
else
    echo "no LV1 in \$L"
fi
pgrep -x "eMotion LV1" >/dev/null && echo "LV1 is RUNNING (pid \$(pgrep -x 'eMotion LV1'))" || echo "LV1 is not running"
[ -w "\$L/Modules" ] && echo "Modules folder: writable by \$(id -un)" || echo "Modules folder: needs sudo (owner \$(stat -f %Su "\$L/Modules" 2>/dev/null))"
ls -1 "\$L/Modules" 2>/dev/null | sed 's/^/  /'
find "\$HOME/Library/Preferences" -maxdepth 3 -name 'LV1.Mixer SoundGrid.preferences' 2>/dev/null | sed 's/^/prefs: /'
P=\$(find "\$HOME/Library/Preferences" -maxdepth 3 -name 'LV1.Mixer SoundGrid.preferences' 2>/dev/null | head -1)
[ -n "\$P" ] && grep -o '<RelativePathFromApp>[^<]*</RelativePathFromApp>' "\$P" | sed 's/^/  surface: /'
[ -d "\$L/Modules/TuneSync_Control.bundle" ] && echo "TuneSync installed: \$(defaults read "\$L/Modules/TuneSync_Control.bundle/Contents/Info" CFBundleShortVersionString)" || echo "TuneSync not installed"
$FIND_DATA
echo "data folder: \$D"; ls -l "\$D" 2>/dev/null | sed 's/^/  /'
REMOTE
    ;;
install)
    Z=$ROOT/dist/mac/TuneSync-$VER-macos.zip
    [ -f "$Z" ] || { echo "no $Z: run tools/build-mac.sh first" >&2; exit 1; }
    "${SSH[@]}" "$M" "rm -rf ~/tunesync-install && mkdir -p ~/tunesync-install && cat > ~/tunesync-install/t.zip" < "$Z"
    remote "$VER" <<REMOTE
cd ~/tunesync-install && ditto -x -k t.zip . && cd "TuneSync-\$1-macos" || exit 1
if pgrep -x "eMotion LV1" >/dev/null; then echo "eMotion LV1 is running: quit it first (not done by this script)"; exit 2; fi
if [ -w "/Applications/Waves/eMotion LV1/Modules" ]; then
    ./install.sh || exit \$?
else
    echo "LV1's Modules folder needs sudo; on the Mac, run:"
    echo "  sudo \$PWD/install.sh"
    exit 3
fi
$FIND_DATA
mkdir -p "\$D"
if [ ! -f "\$D/settings.ini" ]; then
    printf '[tune-remote]\nrtp_enabled=1\nrtp_port=5004\nchannel=16\nscale_cc=20\n' > "\$D/settings.ini"
    echo "seeded \$D/settings.ini (RTP-MIDI on, port 5004, channel 16, scale CC 20)"
fi
REMOTE
    "${SSH[@]}" "$M" "cat > ~/tunesync-install/rtpsend && chmod +x ~/tunesync-install/rtpsend" < "$ROOT/dist/mac/rtpsend"
    ;;
cue)
    [ $# -gt 0 ] || { echo "cue needs cues, e.g. cue cc 20 4 key D" >&2; exit 1; }
    remote "$@" <<REMOTE
~/tunesync-install/rtpsend --ch 16 127.0.0.1 5004 "\$@"; echo "rtpsend exit \$?"
sleep 1
$FIND_DATA
tail -n 8 "\$D/tunesync.log"
REMOTE
    ;;
log)
    remote <<REMOTE
$FIND_DATA
tail -n 40 "\$D/tunesync.log"
REMOTE
    ;;
remove)
    remote "$VER" <<REMOTE
cd ~/tunesync-install/"TuneSync-\$1-macos" 2>/dev/null || { echo "no ~/tunesync-install: nothing installed by this script"; exit 1; }
./install.sh --remove
REMOTE
    ;;
*)
    sed -n '2,22s/^# \{0,1\}//p' "$0"; exit 1 ;;
esac
