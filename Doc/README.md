# Working on Digraph DS

`src/lib.rs` assembles the application in `build_app()`. It installs devy's
hardware video setup and input plugin, creates the editor resources, and
registers the two renderers. `src/main.rs` starts devy's VBlank loop on DS or
runs one frame on the host.

| Module | Responsibility |
| --- | --- |
| `graph.rs` | Vertices, edges, identifiers, and change revisions |
| `analysis.rs` | Analysis catalogue, results, and recomputation |
| `dag.rs` | DAG transformations and bounded feedback-set solvers |
| `editor.rs` | Buttons, stylus actions, selection, and project commands |
| `ui.rs` | Menus, dialogs, themes, pan, and zoom |
| `top2d.rs` | Drawing primitives and upper-screen results |
| `bottom2d.rs` | Interactive canvas and command palette |
| `project.rs` | Versioned serialization, named library, and DS filesystem I/O |
| `stats.rs` | Frame timing and memory statistics |

## Change a graph

```rust
use digraph_ds::graph::Digraph;

let mut graph = Digraph::new();
let a = graph.add_vertex(40, 50).unwrap();
let b = graph.add_vertex(140, 100).unwrap();
graph.add_edge(a, b, true);
graph.move_vertex(b, 160, 120);
```

Use the graph methods when editing. They update the revisions used to
invalidate cached work. A position change refreshes the drawing without
recomputing a topology-only analysis. Changing the public vectors directly
can bypass that tracking.

## Run an analysis

```rust
use digraph_ds::{dag, graph::Digraph};

let mut graph = Digraph::new();
let a = graph.add_vertex(30, 30).unwrap();
let b = graph.add_vertex(90, 60).unwrap();
graph.add_edge(a, b, true);
let result = dag::run(&graph, dag::DagTransformKind::SccCondensation);
assert!(result.exact);
```

The interactive application keeps the selection in `AnalysisState` and runs
`refresh_analysis` after `editor_input`. Results remain separate from the
editable graph until a supported transformation is applied. Check `exact`
and `truncated` before describing a bounded solver's result as optimal.

## Rendering and storage

The app owns its graph renderers. They draw into RAM, then use the DS backend
to upload the visible pixels. Do not add devy's `DsPlugins` to `build_app()`:
that group initializes a different framebuffer layout. The Nintendo paths
are selected with `target_vendor = "nintendo"`; this custom target also belongs
to the Unix family, so `cfg(unix)` does not distinguish it from Linux.

`ProjectSnapshot::capture` records the graph and view state. `project::encode`
and `project::decode` handle the Postcard format without filesystem access.
The save/load functions mount FAT lazily and maintain the named-project index.
If the format changes, update its version deliberately; enum ordering affects
the serialized representation.

For engine APIs and hardware services, use the
[devy programming guide](https://github.com/Vjarkjan/devy/tree/blockds/Doc).
For build checks, see [verification.md](verification.md).
