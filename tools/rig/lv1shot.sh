#!/bin/bash
# lv1shot.sh <out.png> [x y] - (optional click at screen x,y, then) screenshot LV1's main
# window on the rig through the interactive session; prints LV1's windows.
R=${RIG:-user@lv1-host}; P=${RIGPORT:-22}
ARGS="-Shot"; [ -n "${2:-}" ] && ARGS="-X $2 -Y $3 -Shot"
timeout 90 ssh -p $P -o BatchMode=yes $R "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\Users\\you\\hlinter.ps1 -Cmd \"& C:\\Users\\you\\lv1ui.ps1 $ARGS\"" 2>/dev/null | grep -v "^$" | head -12
scp -q -P $P $R:C:/Users/you/lv1ui.png "$1"
