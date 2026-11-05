//! Where the memory of the network step goes (docs/proposals/build-memory.md). A counting allocator gives the live bytes
//! and the peak after each stage, and the parts of the network are dropped one by one to see what each one held.
//!
//!     cargo run --release -p duckosm-core --example mem_probe -- <file.osm.pbf> <driving|walking|cycling> [threads] [off]
//!
//! `off` is a list of steps to switch off, separated by commas: graph, restrictions, merge, ways, connect, speeds, component.
//! Run the same area with one step off and compare the peaks to see what that step costs.
//!
//! It reads the file, builds one network (the default options) and prints MiB. The numbers are the Rust heap of this process,
//! not the WebAssembly memory, but the data structures are the same.

use duckosm_core::filter::Mode;
use duckosm_core::fixes::OsmFixes;
use duckosm_core::network::{build_network_with, Options};
use duckosm_core::pbf;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Relaxed) + l.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size();
                PEAK.fetch_max(now, Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

fn mib(b: usize) -> f64 {
    b as f64 / 1048576.0
}
fn live() -> usize {
    LIVE.load(Relaxed)
}
/// Print the live bytes and the peak since the last call, then reset the peak to the live bytes.
fn stage(name: &str) {
    println!(
        "{name:<34} live {:>8.1} MiB   peak {:>8.1} MiB",
        mib(live()),
        mib(PEAK.load(Relaxed))
    );
    PEAK.store(live(), Relaxed);
}
/// Drop `x` and print how many bytes it held.
fn held<T>(name: &str, x: T, rows: usize) {
    let before = live();
    drop(x);
    let b = before - live();
    let per = if rows > 0 {
        b as f64 / rows as f64
    } else {
        0.0
    };
    println!(
        "  held by {name:<26} {:>8.1} MiB   {per:>7.1} bytes per row ({rows} rows)",
        mib(b)
    );
}

fn hook(name: &str) {
    stage(name);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (path, mode) = (&args[1], args[2].as_str());
    if let Some(t) = args.get(3) {
        rayon::ThreadPoolBuilder::new()
            .num_threads(t.parse().unwrap())
            .build_global()
            .unwrap();
    }
    let mut opts = Options::default();
    for s in args
        .get(4)
        .map_or("", |s| s.as_str())
        .split(',')
        .filter(|s| !s.is_empty())
    {
        match s {
            "graph" => opts.build_graph = false,
            "restrictions" => opts.extract_restrictions = false,
            "merge" => opts.merge_segments = false,
            "ways" => opts.ways_tables = false,
            "connect" => opts.connect_paths = false,
            "speeds" => opts.process_speeds = false,
            "component" => opts.keep_largest_component = false,
            _ => panic!("unknown step {s}"),
        }
    }
    let mode = match mode {
        "driving" => Mode::Driving,
        "walking" => Mode::Walking,
        _ => Mode::Cycling,
    };
    duckosm_core::util::STEP_HOOK.set(hook).ok();
    stage("start");
    let ways = pbf::read_ways(Path::new(path)).unwrap();
    stage("read ways");
    let coords = pbf::read_coords(Path::new(path), &ways).unwrap();
    stage("read coords");
    let mut net =
        build_network_with(&ways, &coords, mode, &opts, &OsmFixes::default(), &[]).unwrap();
    stage("network built");
    println!(
        "edges {}, private {}, nodes {}, ways {}, edge_graph rows {}",
        net.edges.len(),
        net.private_edges.len(),
        net.nodes.len(),
        net.ways.len(),
        net.graph.edge_graph.as_ref().map_or(0, |g| g.len())
    );
    // what each part of the network holds
    let n = net.edges.len();
    let verts: usize = net.edges.iter().map(|e| e.geometry.len()).sum();
    println!("vertices per edge {:.2}", verts as f64 / n as f64);
    // what tight vectors would save: the capacity that the vectors of an edge do not use
    let slack: usize = net
        .edges
        .iter()
        .map(|e| {
            (e.refs.capacity() - e.refs.len()) * 8 + (e.geometry.capacity() - e.geometry.len()) * 16
        })
        .sum();
    let before = live();
    for e in net.edges.iter_mut() {
        e.refs.shrink_to_fit();
        e.geometry.shrink_to_fit();
    }
    println!("  unused capacity in refs and geometry: {:.1} MiB ({:.1} bytes per edge); shrink_to_fit freed {:.1} MiB",
        mib(slack), slack as f64 / n as f64, mib(before.saturating_sub(live())));
    println!(
        "  size_of Edge {} bytes, GraphRow {} bytes",
        std::mem::size_of::<duckosm_core::model::Edge>(),
        std::mem::size_of::<duckosm_core::graph::GraphRow>()
    );
    held("graph", std::mem::take(&mut net.graph), n);
    held(
        "ways (ways and way_nodes)",
        std::mem::take(&mut net.ways),
        n,
    );
    held("edges", std::mem::take(&mut net.edges), n);
    held("private edges", std::mem::take(&mut net.private_edges), 0);
    held("nodes", std::mem::take(&mut net.nodes), 0);
    held("the rest of the network", net, 0);
    held("coords", coords, 0);
    held("ways read (the input)", ways, 0);
    stage("end");
}
