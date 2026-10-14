#!/usr/bin/env bash
set -euo pipefail
digraph_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
action="${1:-rom}"
if (($#)); then shift; fi
case "$action" in
    check|build|clippy|rom) ;;
    *) echo 'Usage: nds.sh {check|build|clippy|rom} [Cargo arguments]' >&2; exit 2 ;;
esac
export BLOCKSDS="${BLOCKSDS:-/opt/wonderful/thirdparty/blocksds/core}"
export WONDERFUL_TOOLCHAIN="${WONDERFUL_TOOLCHAIN:-/opt/wonderful}"
export CARGO_TARGET_ARMV5TE_NINTENDO_DS_LINKER="$digraph_root/scripts/nds-linker.sh"
if [[ ! -f "$BLOCKSDS/libs/libnds/lib/libnds9.a" ]]; then
    echo "BlocksDS was not found at $BLOCKSDS; set BLOCKSDS to your SDK directory." >&2
    exit 1
fi
cd "$digraph_root"
cargo_action="$action"
[[ "$action" != rom ]] || cargo_action=build
extra=()
[[ "$action" != clippy ]] || extra=(-- -D warnings)
if [[ "$action" == rom ]]; then
    mkdir -p output
    manifest_log="$(mktemp)"
    trap 'rm -f "$manifest_log"' EXIT
    cargo "+${NDS_TOOLCHAIN:-nightly}" "$cargo_action" --locked --release \
        --config .cargo/nds.toml --target "$digraph_root/armv5te-nintendo-ds.json" \
        --bin digraph-ds --message-format=json-render-diagnostics "$@" > "$manifest_log"
    python3 scripts/package_rom.py "$manifest_log"
else
    cargo "+${NDS_TOOLCHAIN:-nightly}" "$cargo_action" --locked --release \
        --config .cargo/nds.toml --target "$digraph_root/armv5te-nintendo-ds.json" \
        --bin digraph-ds "$@" "${extra[@]}"
fi
