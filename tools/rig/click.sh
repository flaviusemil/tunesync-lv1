#!/bin/bash
# click.sh <out.png> [x y [-Double]] - click.ps1 on the rig (physical pixels), then fetch the
# screenshot and a half-size copy (<out>-s.png) to look at.
R=${RIG:-user@lv1-host}; P=${RIGPORT:-22}
OUT=$1; shift
A="-NoClick"; [ $# -ge 2 ] && A="-X $1 -Y $2 $3"
timeout 90 ssh -p $P -o BatchMode=yes $R "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\Users\\you\\hlinter.ps1 -Cmd \"& C:\\Users\\you\\tunesync\\click.ps1 $A\"" 2>/dev/null | grep -v '^$'
scp -q -P $P $R:C:/Users/you/tunesync/screen.png "$OUT"
python3 -c "
from PIL import Image; im=Image.open('$OUT'); im.resize((im.width//2, im.height//2)).save('${OUT%.png}-s.png')"
