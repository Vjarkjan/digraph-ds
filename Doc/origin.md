# Origin

This repository continues the Digraph Explorer 0.5.10 application present in
the local `kts-nds` working tree. That tree includes local application changes
beyond its upstream Git revision; the upstream game alone is not the source
of this editor.

The extraction starts from devy's integrated `examples/digraph` at commit
`772db82`. Its nine modules retain the graph algorithms, interaction, two
renderers, diagnostics, and project persistence. Imports now refer to this
crate's modules, and application assembly is shared through `build_app()`.
The DS target specification, build helpers, and icon also come from that
integration. The crate depends on a fixed devy Git revision and records the
resolved dependencies in `Cargo.lock`.

The reusable runtime and asset tools remain in
[devy](https://github.com/Vjarkjan/devy). Devy retains the earlier application
snapshot as an integration example; application development belongs here.
Neither repository incorporates `cred`.

The hardware foundation comes from Christopher J. Miller's
[kts-nds](https://github.com/ChristopherJMiller/kts-nds). Devy's
[source inventory](https://github.com/Vjarkjan/devy/blob/blockds/Doc/source-inventory.json)
records the original paths and hashes. This extraction does not add a new
license grant; the packages remain unpublished on crates.io.
