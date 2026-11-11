//! Compare `read_osm` (Rust) with `ST_READOSM` (DuckDB spatial) in the same DuckDB, with the same settings:
//! `read_osm_compare <file.osm.pbf> <dir> <rust|st> [memory_limit] [threads]`. It makes `raw.*` in `<dir>/<which>.duckdb`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (pbf, dir, which) = (&a[1], &a[2], a[3].as_str());
    let mem = a.get(4).cloned().unwrap_or("4GB".into());
    let threads = a.get(5).cloned().unwrap_or("20".into());
    let con = duckdb::Connection::open(format!("{dir}/{which}.duckdb")).unwrap();
    con.execute_batch(&format!(
        "SET memory_limit = '{mem}'; SET threads = {threads}; SET preserve_insertion_order = false; CREATE SCHEMA IF NOT EXISTS raw;"
    ))
    .unwrap();
    let sql: Vec<String> = if which == "rust" {
        duckosm_duckdb::read_osm::register(&con).unwrap();
        duckosm_duckdb::read_osm::raw_sql(pbf).split(";\n").skip(1).map(String::from).collect()
    } else {
        con.execute_batch("INSTALL spatial; LOAD spatial;").unwrap();
        let o = format!("ST_READOSM('{pbf}')");
        vec![
            format!("CREATE OR REPLACE TABLE raw.nodes AS SELECT id AS osm_id, lat, lon, tags FROM {o} WHERE kind = 'node' AND lat IS NOT NULL AND lon IS NOT NULL"),
            format!("CREATE OR REPLACE TABLE raw.ways AS SELECT id AS osm_id, tags, refs FROM {o} WHERE kind = 'way' AND len(refs) >= 2"),
            format!("CREATE OR REPLACE TABLE raw.relations AS SELECT id AS osm_id, tags, refs, ref_roles, ref_types FROM {o} WHERE kind = 'relation'"),
        ]
    };
    let all = std::time::Instant::now();
    for s in sql {
        let t = std::time::Instant::now();
        con.execute_batch(&s).unwrap();
        let table = s.split_whitespace().nth(4).unwrap().to_string();
        println!("{which}: {table} {:.1} s", t.elapsed().as_secs_f64());
    }
    println!("{which}: all {:.1} s", all.elapsed().as_secs_f64());
}
