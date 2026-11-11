//! The load step written directly into DuckDB: `load_duckdb <file.osm.pbf> <out.duckdb> [budget bytes] [threads]`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let budget = a.get(3).map_or(1_000_000_000, |s| s.parse().unwrap());
    let threads = a.get(4).map(|s| s.parse().unwrap());
    let t = std::time::Instant::now();
    let s = duckosm_duckdb::load_raw_duckdb(a[1].as_ref(), a[2].as_ref(), budget, threads).unwrap();
    println!(
        "rows {:?}, {} blocks, at most {} in work: {:.1} s",
        s.rows,
        s.blocks,
        s.most_in_flight,
        t.elapsed().as_secs_f64()
    );
}
