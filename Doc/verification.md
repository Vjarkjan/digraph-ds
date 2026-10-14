# Validation

Checks performed on 14 October 2026 against the devy revision pinned in
`Cargo.toml`. Cargo resolves the engine from GitHub; `digraph-ds` is the only
local package in the dependency graph.

Environment: Rust 1.95.0, nightly 1.100.0 (`420ed2a0c`, 18 September 2026),
BlocksDS with libnds 1.8.0, ndstool 1.23.0, and DeSmuME 0.9.13 in DS mode.

The eight inherited graph and drawing tests pass, as do both examples in the
programming guide. The host executable runs the assembled application and
verifies that the starter graph has five vertices and six edges.

Formatting and Clippy pass on the host; ARM9 Clippy also passes with warnings
treated as errors. The release ARM9 executable links and packages into
`output/digraph-ds.nds`, including the application icon. DeSmuME boots that ROM
and displays the graph on both screens; [screenshot.png](screenshot.png) is
the captured startup view.

The tested ROM has SHA-256
`e6d545e1f9faad1ccde8c657f26a8073fee049ce6f72d7b37d8da313f2ba4441`.

```bash
cargo fmt --all --check
cargo test --locked
cargo run --locked
cargo clippy --locked --all-targets -- -D warnings
bash scripts/nds.sh clippy
bash scripts/nds.sh rom
```

Host execution exercises the editor's schedules and logic with hardware stubs.
It does not exercise VRAM transfers, touchscreen hardware, or FAT writes.
No physical DS/DSi, DSi-mode execution, or persistent SD/DLDI save/load has been
verified for this extraction.

To run the engine's emulator startup check against this app's ROM:

```bash
python3 ../devy/scripts/emulator-smoke.py \
  --emulator /path/to/desmume-cli --xvfb /path/to/Xvfb \
  --examples digraph --digraph-rom "$PWD/output/digraph-ds.nds"
```

That optional check requires a devy checkout, DeSmuME CLI with SDL2, Xvfb,
xdotool, ImageMagick, a C compiler, and Python with Pillow. Normal ROM builds
only need the dependencies listed in the README.
