//! Compare two features tables of QuackOSM's form (feature_id, tags, geometry), each a GeoParquet file or the table
//! main.quackosm of a .duckdb file, for example the output of
//! `duckosm-rust quack` and the one of QuackOSM (docs/rust/quack.md):
//!
//!     cargo run --release --example quack_diff -- area_quack.duckdb quackosm.parquet
//!
//! It shows the rows of each kind, the ids that are only in one file, the rows with other tags and the rows with
//! another geometry (not ST_Equals), with up to 5 examples of each difference.
use duckdb::Connection;

fn main() -> Result<(), duckdb::Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && args.len() != 4 {
        eprintln!("usage: quack_diff <a.parquet> <b.parquet> [feature_id: show its two geometries]");
        std::process::exit(2);
    }
    if let Some(id) = args.get(3) {
        let con = Connection::open_in_memory()?;
        con.execute_batch("INSTALL spatial; LOAD spatial;")?;
        for f in &args[1..3] {
            let wkt: String = con.query_row(
                &format!(
                    "SELECT ST_AsText(geometry::GEOMETRY) || ' area ' || ST_Area(geometry::GEOMETRY) FROM read_parquet('{}') WHERE feature_id = ?",
                    f.replace('\'', "''")
                ),
                [id],
                |r| r.get(0),
            )?;
            println!("{f}: {wkt}\n");
        }
        return Ok(());
    }
    let q = |s: &str| s.replace('\'', "''");
    let con = Connection::open_in_memory()?;
    // a .duckdb file: its table main.quackosm (attached as da or db); else a GeoParquet file
    let source = |f: &str, name: &str| -> Result<String, duckdb::Error> {
        if f.ends_with(".duckdb") {
            con.execute_batch(&format!("ATTACH '{}' AS {name} (READ_ONLY);", q(f)))?;
            Ok(format!("{name}.main.quackosm"))
        } else {
            Ok(format!("read_parquet('{}')", q(f)))
        }
    };
    con.execute_batch("INSTALL spatial; LOAD spatial;")?;
    let (sa, sb) = (source(&args[1], "da")?, source(&args[2], "db")?);
    con.execute_batch(&format!(
        "CREATE TEMP TABLE a AS SELECT feature_id, tags, geometry::GEOMETRY AS geometry FROM {sa};
         CREATE TEMP TABLE b AS SELECT feature_id, tags, geometry::GEOMETRY AS geometry FROM {sb};
         CREATE TEMP TABLE j AS SELECT a.feature_id, a.tags = b.tags AS same_tags,
             ST_AsWKB(a.geometry) = ST_AsWKB(b.geometry) AS same_wkb,
             ST_Equals(a.geometry, b.geometry) AS same_geometry,
             ST_GeometryType(a.geometry)::VARCHAR AS ga, ST_GeometryType(b.geometry)::VARCHAR AS gb
         FROM a JOIN b USING (feature_id);"
    ))?;
    let show = |title: &str, sql: &str| -> Result<(), duckdb::Error> {
        println!("## {title}");
        let mut st = con.prepare(sql)?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let n = r.as_ref().column_count();
            let cells: Vec<String> = (0..n)
                .map(|i| r.get::<_, duckdb::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default())
                .collect();
            println!("  {}", cells.join(" | "));
        }
        Ok(())
    };
    show(
        "rows of each kind: kind | a | b",
        "SELECT kind, count(*) FILTER (WHERE s = 'a'), count(*) FILTER (WHERE s = 'b') FROM (
             SELECT split_part(feature_id, '/', 1) AS kind, 'a' AS s FROM a
             UNION ALL SELECT split_part(feature_id, '/', 1), 'b' FROM b) GROUP BY 1 ORDER BY 1",
    )?;
    show(
        "only in a: count",
        "SELECT count(*) FROM a ANTI JOIN b USING (feature_id)",
    )?;
    show(
        "only in a: examples",
        "SELECT feature_id, tags FROM a ANTI JOIN b USING (feature_id) ORDER BY feature_id LIMIT 5",
    )?;
    show(
        "only in b: count",
        "SELECT count(*) FROM b ANTI JOIN a USING (feature_id)",
    )?;
    show(
        "only in b: examples",
        "SELECT feature_id, tags FROM b ANTI JOIN a USING (feature_id) ORDER BY feature_id LIMIT 5",
    )?;
    show(
        "rows in both: all | same tags | same WKB | ST_Equals | same geometry type",
        "SELECT count(*), count(*) FILTER (WHERE same_tags), count(*) FILTER (WHERE same_wkb),
             count(*) FILTER (WHERE same_geometry), count(*) FILTER (WHERE ga = gb) FROM j",
    )?;
    show(
        "other tags: examples",
        "SELECT j.feature_id, a.tags, b.tags FROM j JOIN a USING (feature_id) JOIN b USING (feature_id)
         WHERE NOT j.same_tags ORDER BY 1 LIMIT 5",
    )?;
    show(
        "other geometry (not ST_Equals): kind | a type | b type | rows | largest area of the difference / area of b",
        "SELECT split_part(j.feature_id, '/', 1), ga, gb, count(*),
             max(ST_Area(ST_SymDifference(a.geometry, b.geometry)) / nullif(ST_Area(b.geometry), 0))
         FROM j JOIN a USING (feature_id) JOIN b USING (feature_id)
         WHERE NOT j.same_geometry GROUP BY ALL ORDER BY 4 DESC",
    )?;
    show(
        "other geometry (not ST_Equals): examples",
        "SELECT feature_id, ga, gb FROM j WHERE NOT same_geometry ORDER BY 1 LIMIT 5",
    )?;
    Ok(())
}
