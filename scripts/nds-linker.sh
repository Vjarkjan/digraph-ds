#!/usr/bin/env bash
set -euo pipefail
: "${BLOCKSDS:?Define BLOCKSDS con la ruta del SDK}"
devy_wonderful="${WONDERFUL_TOOLCHAIN:-/opt/wonderful}"
devy_cc="${DEVY_CC:-$devy_wonderful/toolchain/gcc-arm-none-eabi/bin/arm-none-eabi-gcc}"
exec "$devy_cc" -mthumb -mcpu=arm946e-s+nofp \
    "-specs=$BLOCKSDS/sys/crts/ds_arm9.specs" \
    "-L$BLOCKSDS/libs/libnds/lib" "-L$BLOCKSDS/libs/maxmod/lib" "$@" \
    -Wl,--start-group -lmm9 -lnds9 -lc -lgcc -Wl,--end-group
