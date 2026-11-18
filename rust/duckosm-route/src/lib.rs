//! Routing on a built duckOSM database in Rust (docs/design/all_in_rust.md, group E). The graph is in memory, with the
//! order of networkx's graph, so the routes (also when two routes cost the same) are those of duckOSM's Python code.

pub mod cli;
pub mod directions;
pub mod graphml;
pub mod lanes;
pub mod points;
pub mod route;

pub use duckosm_router::graph;

pub type Result<T> = std::result::Result<T, String>;

pub(crate) fn err(e: duckdb::Error) -> String {
    e.to_string()
}
