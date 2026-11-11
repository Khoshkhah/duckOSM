//! The speed of the ways pass without a table: `read_osm_scan <file.osm.pbf> <rust|st> [threads]`. The query reads
//! every column (the sum of the refs and of the tags), so both sides do all the work of the rows.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (pbf, which) = (&a[1], a[2].as_str());
    let threads = a.get(3).cloned().unwrap_or("20".into());
    let con = duckdb::Connection::open_in_memory().unwrap();
    con.execute_batch(&format!("SET threads = {threads};")).unwrap();
    let from = if which == "rust" {
        duckosm_duckdb::read_osm::register(&con).unwrap();
        format!("read_osm('{pbf}', 'way')")
    } else {
        con.execute_batch("INSTALL spatial; LOAD spatial;").unwrap();
        format!("(SELECT * FROM ST_READOSM('{pbf}') WHERE kind = 'way' AND len(refs) >= 2)")
    };
    let t = std::time::Instant::now();
    let (n, r, k): (i64, i64, i64) = con
        .query_row(&format!("SELECT count(*), sum(len(refs)), sum(cardinality(tags)) FROM {from}"), [], |x| {
            Ok((x.get(0)?, x.get(1)?, x.get(2)?))
        })
        .unwrap();
    println!("{which}: {n} ways, {r} refs, {k} tags: {:.1} s", t.elapsed().as_secs_f64());
}
