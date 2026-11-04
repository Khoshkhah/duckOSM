//! Write the network as a DuckDB file. The Parquet files that `duckosm-parquet` wrote are loaded
//! with DuckDB itself, so the types are the types of duckOSM (BIGINT, FLOAT, BIGINT[], GEOMETRY).

use duckdb::Connection;
use std::path::Path;

const MACROS: &str = "
CREATE OR REPLACE MACRO main.edge_id_hash(osm_id, source, target) AS
    (hash(osm_id::BIGINT, source::BIGINT, target::BIGINT) >> 1)::BIGINT;
CREATE OR REPLACE MACRO main.edge_id_hash_v1(osm_id, source, target, is_reverse) AS
    (hash(osm_id::BIGINT, source::BIGINT, target::BIGINT, is_reverse::BOOLEAN) >> 1)::BIGINT;
";

pub fn write_database(db: &Path, parquet_dir: &Path, modes: &[&str]) -> Result<(), String> {
    let _ = std::fs::remove_file(db);
    let con = Connection::open(db).map_err(|e| e.to_string())?;
    for mode in modes {
        write_schema(&con, parquet_dir, mode, MODE_TABLES)?;
    }
    write_schema(
        &con,
        parquet_dir,
        "main",
        &["global_junctions", "boundary", "visualization_metadata"],
    )?;
    write_schema(&con, parquet_dir, "mm", &["transfers"])?;
    write_schema(&con, parquet_dir, "bus", &["routes", "route_edges"])?;
    if parquet_dir
        .join("edge_levels_visualization.parquet")
        .exists()
    {
        write_schema(
            &con,
            parquet_dir,
            "visualization",
            &["edge_levels", "edge_levels_meta"],
        )?;
    }
    // the elevation step: duckOSM's columns on the nodes and the edges, and main.elevation_metadata
    for mode in modes {
        let (nz, ez) = (
            parquet_dir.join(format!("elevation_nodes_{mode}.parquet")),
            parquet_dir.join(format!("elevation_edges_{mode}.parquet")),
        );
        if nz.exists() && ez.exists() {
            con.execute_batch(&format!(
                "ALTER TABLE {mode}.nodes ADD COLUMN ele DOUBLE;
                 UPDATE {mode}.nodes n SET ele = t.ele FROM read_parquet('{}') t WHERE t.node_id = n.node_id;
                 ALTER TABLE {mode}.edges ADD COLUMN z_from DOUBLE;
                 ALTER TABLE {mode}.edges ADD COLUMN z_to DOUBLE;
                 ALTER TABLE {mode}.edges ADD COLUMN z_vertices DOUBLE[];
                 UPDATE {mode}.edges e SET z_from = t.z_from, z_to = t.z_to, z_vertices = t.z_vertices
                   FROM read_parquet('{}') t WHERE t.edge_id = e.edge_id;",
                nz.display(),
                ez.display()
            ))
            .map_err(|e| e.to_string())?;
        }
    }
    if parquet_dir.join("elevation_metadata_main.parquet").exists() {
        write_schema(&con, parquet_dir, "main", &["elevation_metadata"])?;
    }
    if parquet_dir.join("transfers_mm.parquet").exists() {
        con.execute_batch(&mm_edges(modes))
            .map_err(|e| e.to_string())?;
    }
    con.execute_batch(MACROS).map_err(|e| e.to_string())
}

/// The tables of a mode, when their files exist (the options decide which ones exist).
const MODE_TABLES: &[&str] = &[
    "edges",
    "private_edges",
    "nodes",
    "virtual_nodes",
    "ways",
    "way_nodes",
    "edge_id_map",
    "edge_graph",
    "turn_permission",
    "turn_restrictions",
    "turn_path_restrictions",
    "lane_profile",
];

/// The SELECT of a table from its file, with duckOSM's names and types. The same rules are in the page
/// (crates/duckosm-wasm/www/app/duck.js) and in its load.sql.
pub fn select(table: &str) -> &'static str {
    match table {
        "nodes" | "virtual_nodes" => "SELECT node_id, geometry AS geom",
        "boundary" => "SELECT * EXCLUDE (geometry), geometry AS geom",
        "visualization_metadata" => {
            "SELECT boundary_geojson::JSON AS boundary_geojson, center_lat, center_lon, initial_zoom, timezone, built_at, duckosm_version"
        }
        _ => "SELECT *",
    }
}

/// duckOSM's view `mm.edges`: the edges of all modes with a `mode` column, the modes in name order.
pub fn mm_edges(modes: &[&str]) -> String {
    let mut m = modes.to_vec();
    m.sort_unstable();
    let parts: Vec<String> = m
        .iter()
        .map(|m| format!("SELECT '{m}' AS mode, edge_id, source, target, cost_s, length_m, highway, name, geometry FROM {m}.edges"))
        .collect();
    format!(
        "CREATE OR REPLACE VIEW mm.edges AS {};",
        parts.join(" UNION ALL ")
    )
}

fn write_schema(
    con: &Connection,
    parquet_dir: &Path,
    schema: &str,
    tables: &[&str],
) -> Result<(), String> {
    con.execute_batch(&format!("CREATE SCHEMA IF NOT EXISTS {schema};"))
        .map_err(|e| e.to_string())?;
    for table in tables {
        let file = parquet_dir.join(format!("{table}_{schema}.parquet"));
        if file.exists() {
            let f = file.display().to_string().replace('\'', "''");
            con.execute_batch(&format!(
                "CREATE TABLE {schema}.{table} AS {} FROM read_parquet('{f}');",
                select(table)
            ))
            .map_err(|e| format!("{schema}.{table}: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use duckdb::Connection;
    use duckosm_core::hash;

    /// The edge_id of duckosm-core must equal the hash() of the DuckDB inside this program.
    /// If a new DuckDB changes hash(), this test fails.
    #[test]
    fn hash_equals_the_bundled_duckdb() {
        let con = Connection::open_in_memory().unwrap();
        let version: String = con.query_row("SELECT version()", [], |r| r.get(0)).unwrap();
        for (a, b, c) in [
            (1i64, 2i64, 3i64),
            (123456789, 987654321, -555),
            (-1, 0, i64::MAX),
            (4230114, 1868723946, 1080509801),
        ] {
            let want: u64 = con
                .query_row(
                    "SELECT hash(?::BIGINT, ?::BIGINT, ?::BIGINT)",
                    [a, b, c],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                hash::hash3(a, b, c),
                want,
                "DuckDB {version}: hash({a}, {b}, {c})"
            );
        }
        for s in [
            "[1, 2, 3]",
            "",
            "[10, 2]",
            "a longer text with more than sixteen bytes",
        ] {
            let want: u64 = con
                .query_row("SELECT hash(?::VARCHAR)", [s], |r| r.get(0))
                .unwrap();
            assert_eq!(hash::hash_str(s), want, "DuckDB {version}: hash('{s}')");
        }
        eprintln!("DuckDB {version}: the hash agrees");
    }
}
