#!/usr/bin/env python3
"""Package the Cargo ARM9 artifact with the BlocksDS ARM7 and Digraph icon."""
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
sdk = Path(os.environ["BLOCKSDS"])
arm7 = Path(os.environ.get("DIGRAPH_ARM7", sdk / "sys/arm7/main_core/arm7_maxmod.elf"))
executable = None
for line in Path(sys.argv[1]).read_text().splitlines():
    item = json.loads(line)
    if (item.get("reason") == "compiler-artifact"
            and item["target"]["name"] == "digraph-ds"
            and item.get("executable")):
        executable = Path(item["executable"])
if executable is None:
    raise SystemExit("Cargo did not produce the digraph-ds executable.")

output = root / "output/digraph-ds.nds"
output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run([
    str(sdk / "tools/ndstool/ndstool"), "-c", str(output),
    "-9", str(executable), "-7", str(arm7),
    "-g", "DGPH", "00", "DIGRAPH DS",
    "-b", str(root / "assets/icon.bmp"), "Digraph DS;Graph editor;Vjarkjan",
], check=True)
print(f"ROM: {output}")
