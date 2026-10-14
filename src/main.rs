#![cfg_attr(target_vendor = "nintendo", no_std)]
#![cfg_attr(target_vendor = "nintendo", no_main)]

#[cfg(target_vendor = "nintendo")]
#[unsafe(no_mangle)]
pub extern "C" fn main() -> core::ffi::c_int {
    devy::run(digraph_ds::build_app())
}

#[cfg(not(target_vendor = "nintendo"))]
fn main() {
    let mut app = digraph_ds::build_app();
    app.update();
    let graph = app.world().resource::<digraph_ds::graph::Digraph>();
    assert_eq!((graph.vertices.len(), graph.edges.len()), (5, 6));
    println!(
        "Digraph DS initialized: {} vertices, {} edges.\nBuild the ROM with bash scripts/nds.sh rom to use the graphical editor.",
        graph.vertices.len(),
        graph.edges.len()
    );
}
