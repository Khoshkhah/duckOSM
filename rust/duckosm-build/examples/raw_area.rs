//! The raw tables of a whole `.osm.pbf` file with no build (`duckosm_parquet::write_area_raw`, an empty patch): the time and
//! the memory of the raw writer alone. `cargo run --release --example raw_area -- FILE.osm.pbf OUT_DIR`; OUT_DIR `-` writes
//! the rows to nowhere (a measurement keeps no files; only the temporary files of the join are on disk).
use duckosm_core::raw::{Raw, RawPatch};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (pbf, out) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let sink = out.as_os_str() == "-";
    if !sink {
        std::fs::create_dir_all(&out).unwrap();
    }
    let open = || std::fs::File::open(&pbf).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| e.to_string());
    let raw = Raw::default();
    let patch = RawPatch { fixed: &raw, read: Default::default(), net: Vec::new() };
    let file = |n: &str| -> Box<dyn std::io::Write + Send> {
        if sink {
            Box::new(std::io::sink())
        } else {
            Box::new(std::fs::File::create(out.join(n)).unwrap())
        }
    };
    let files = [file("raw_nodes.parquet"), file("raw_ways.parquet"), file("raw_relations.parquet")];
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now);
    let _ = duckosm_core::util::STEP_HOOK.set(|name| eprintln!("  {:>7.1} s  {name}", START.get().unwrap().elapsed().as_secs_f64()));
    let t = std::time::Instant::now();
    let n = duckosm_parquet::write_area_raw(open, &patch, files, 64 << 20).unwrap();
    eprintln!("raw: {} nodes, {} ways, {} relations in {:.1} s", n[0], n[1], n[2], t.elapsed().as_secs_f64());
}
