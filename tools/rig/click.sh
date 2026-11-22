#!/bin/bash
# click.sh <out.png> [x y [-Double]] - click.ps1 on the rig (physical pixels), then fetch the
# screenshot and a half-size copy (<out>-s.png) to look at.
R=${RIG:?set RIG=user@host of the LV1 machine}; P=${RIGPORT:-22}
# INTER: a script on the rig that runs a command in the desktop session (an interactive
# scheduled task) and prints its output; SSH sessions cannot see windows.
I=${RIG_INTER:?set RIG_INTER to the desktop-session runner on the rig}
OUT=$1; shift
A="-NoClick"; [ $# -ge 2 ] && A="-X $1 -Y $2 $3"
timeout 90 ssh -p $P -o BatchMode=yes $R "powershell -NoProfile -ExecutionPolicy Bypass -File $I -Cmd \"& \$env:USERPROFILE\\tunesync\\click.ps1 $A\"" 2>/dev/null | grep -v '^$'
scp -q -P $P $R:tunesync/screen.png "$OUT"
python3 -c "
from PIL import Image; im=Image.open('$OUT'); im.resize((im.width//2, im.height//2)).save('${OUT%.png}-s.png')"
