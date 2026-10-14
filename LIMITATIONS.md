# Restrictions

Digraph DS provides a 2D editor on both screens, touch input, graph analysis,
and project storage through BlocksDS. These are the constraints of that
implementation.

- Each screen has a 256 × 192 drawing area. Touch input comes from the bottom
  screen and tracks one contact.
- The graph model accepts at most 200 vertices and 20,000 edges. It rejects
  self-loops. These are model caps, not a promise that every graph at the caps
  fits comfortably in memory or can be analyzed within one frame.
- The editor stores one edge per unordered pair. Adding the opposite directed
  edge converts the pair to an undirected edge rather than keeping two arcs.
- Edges have no numeric weights. Shortest paths and the implemented cut and
  flow analyses use unweighted or unit-capacity interpretations.
- Exact minimum feedback arc search is bounded to 24 vertices and 128 arcs;
  exact feedback vertex search is bounded to 28 vertices. Both have a budget
  of 60,000 search nodes. When a bound is reached, the result is a valid
  heuristic transformation, not a guaranteed minimum; the result records
  `exact = false` and `truncated = true`.
- Cycle enumeration shows at most 64 cycles and stops after 250,000 search
  steps. Unilateral component search likewise has result and work budgets.
  A bounded result cannot establish that no further results exist.
- Analyses run on the ARM9 update thread. Expensive work can delay a frame;
  a size limit alone does not guarantee a frame rate.
- The app's renderers reserve Main BG3 in VRAM B and Sub BG3 in VRAM C. Other
  plugins cannot simultaneously reuse those regions. Use the application's
  `build_app()` configuration when extending the editor.
- Writable storage depends on the loader exposing SD or DLDI. Saves are not
  atomic across power loss. Loading rejects files larger than 512 KiB and
  project versions other than the supported version 4.
- The host executable runs the application without a window. Its DS storage
  functions report `UnsupportedHost`; use `project::encode` and
  `project::decode` to work with the format on a computer.

[Doc/verification.md](Doc/verification.md) records execution checks separately
from these restrictions. Engine-level hardware contracts are documented in
[devy's LIMITATIONS.md](https://github.com/Vjarkjan/devy/blob/blockds/LIMITATIONS.md).
