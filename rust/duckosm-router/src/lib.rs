//! The routing algorithms of duckOSM, with plain data in and out (no DuckDB, no threads), so that the same code runs in the
//! program, in Python (through the module) and in a page (WebAssembly). The graph keeps the order of networkx's graph, so a
//! route equals the route of duckOSM's Python code, also when two routes cost the same.

pub mod graph;

pub use graph::Graph;

pub type Result<T> = std::result::Result<T, String>;
pub mod geom;
pub mod multimodal;
pub mod points;
