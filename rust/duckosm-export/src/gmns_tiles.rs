//! GMNS by tiles (docs/design/gmns_by_tiles.md). An area with nodes in two or more map tiles is made one tile at a time:
//! the edges of the tile and of a band around it go into a small source db, the GMNS steps of [`crate::gmns`] run on it
//! without a change, and the GMNS db keeps only the rows that the tile owns. The memory is that of one tile and its band.

use crate::gmns::{Ctx, GmnsOptions};
use crate::Result;
use pbfcut::tiles::Tile;
use std::path::Path;

/// The width of the band around a tile, in metres: the context of the steps that look at the roads near a road (the
/// opposite carriageway, the lanes that a one-way road takes over, the crossings, the junctions of netconvert).
pub const HALO_M: f64 = 2000.0;

/// The tile x, y (zoom `z`) of the point `lon`, `lat` in SQL: the formula of [`Tile::of`].
fn tile_sql(z: u8, lon: &str, lat: &str) -> (String, String) {
    let n = 1u64 << z;
    (
        format!("least(greatest(floor(({lon} + 180.0) / 360.0 * {n}), 0), {})::BIGINT", n - 1),
        format!(
            "least(greatest(floor((1.0 - asinh(tan(radians(least(greatest({lat}, -85.05112878), 85.05112878)))) / pi()) / 2.0 * {n}), 0), {})::BIGINT",
            n - 1
        ),
    )
}

/// The tiles of zoom `z` that have nodes of the modes, south to north, then west to east (the order of [`pbfcut::tiles::tiles`]).
pub(crate) fn tiles_of(c: &Ctx, modes: &[String], z: u8) -> Result<Vec<Tile>> {
    let (x, y) = tile_sql(z, "ST_X(geom)", "ST_Y(geom)");
    let union = modes.iter().map(|m| format!("SELECT {x} AS x, {y} AS y FROM s.{m}.nodes WHERE geom IS NOT NULL")).collect::<Vec<_>>().join(" UNION ");
    let mut st = c.con.prepare(&format!("SELECT DISTINCT x, y FROM ({union}) ORDER BY y DESC, x")).map_err(|e| e.to_string())?;
    let v = st.query_map([], |r| Ok(Tile { z, x: r.get::<_, i64>(0)? as u32, y: r.get::<_, i64>(1)? as u32 })).map_err(|e| e.to_string())?;
    v.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())
}

/// The box of the tile and its band: west, south, east, north.
fn halo_box(t: Tile) -> (f64, f64, f64, f64) {
    let (w, s, e, n) = t.bounds();
    let dy = HALO_M / 111_320.0;
    let dx = dy / s.abs().max(n.abs()).to_radians().cos().max(1e-9);
    (w - dx, s - dy, e + dx, n + dy)
}

/// Write the source db of tile `t` to `path`: the rows of `s` that the GMNS steps read, in the order of the source.
fn tile_source(c: &Ctx, path: &Path, t: Tile, present: &[String], has_raw: bool) -> Result<()> {
    let (w, s, e, n) = halo_box(t);
    let in_box = |g: &str| format!("({g} IS NULL OR (ST_XMax({g}) >= {w} AND ST_XMin({g}) <= {e} AND ST_YMax({g}) >= {s} AND ST_YMin({g}) <= {n}))");
    c.exec(&format!("ATTACH '{}' AS ts", path.display().to_string().replace('\'', "''")))?;
    let tables = |schema: &str| -> Result<Vec<String>> {
        c.strings(&format!("SELECT table_name FROM duckdb_tables() WHERE database_name = 's' AND schema_name = '{schema}'"))
    };
    let mut osm = Vec::new();
    for m in present {
        let have = tables(m)?;
        let private = have.iter().any(|x| x == "private_edges");
        c.exec(&format!(
            "CREATE SCHEMA ts.{m};
             CREATE TABLE ts.{m}.edges AS SELECT * FROM s.{m}.edges WHERE {} ORDER BY rowid;",
            in_box("geometry")
        ))?;
        if private {
            c.exec(&format!("CREATE TABLE ts.{m}.private_edges AS SELECT * FROM s.{m}.private_edges WHERE {} ORDER BY rowid", in_box("geometry")))?;
        }
        let all = if private { format!("ts.{m}.edges UNION ALL SELECT edge_id, source, target FROM ts.{m}.private_edges") } else { format!("ts.{m}.edges") };
        c.exec(&format!(
            "CREATE TEMP TABLE _te AS SELECT edge_id FROM (SELECT edge_id, source, target FROM {all});
             CREATE TEMP TABLE _tn AS SELECT source AS node_id FROM (SELECT edge_id, source, target FROM {all}) UNION SELECT target FROM (SELECT edge_id, source, target FROM {all})"
        ))?;
        osm.push(format!("SELECT abs(osm_id) AS osm_id FROM ts.{m}.edges"));
        for (table, filter) in [
            ("nodes", "node_id IN (SELECT node_id FROM _tn)".to_string()),
            ("virtual_nodes", "node_id IN (SELECT node_id FROM _tn)".to_string()),
            ("edge_graph", "from_edge IN (SELECT edge_id FROM _te) AND to_edge IN (SELECT edge_id FROM _te)".to_string()),
            ("turn_permission", "from_edge IN (SELECT edge_id FROM _te) OR to_edge IN (SELECT edge_id FROM _te)".to_string()),
            ("turn_restrictions", "from_edge_id IN (SELECT edge_id FROM _te) OR to_edge_id IN (SELECT edge_id FROM _te)".to_string()),
            ("turn_path_restrictions", "from_edge IN (SELECT edge_id FROM _te) OR to_edge IN (SELECT edge_id FROM _te)".to_string()),
            ("lane_profile", "edge_id IN (SELECT edge_id FROM _te)".to_string()),
        ] {
            if have.iter().any(|x| x == table) {
                c.exec(&format!("CREATE TABLE ts.{m}.{table} AS SELECT * FROM s.{m}.{table} WHERE {filter} ORDER BY rowid"))?;
            }
        }
        c.exec("DROP TABLE _te; DROP TABLE _tn")?;
    }
    for table in tables("main")? {
        if table == "boundary" || table == "visualization_metadata" {
            c.exec(&format!("CREATE TABLE ts.main.{table} AS SELECT * FROM s.main.{table} ORDER BY rowid"))?;
        }
    }
    if has_raw {
        // the ways of the edges and the ways with a node in the box (footways and crossings near the roads), with all
        // their nodes
        c.exec(&format!(
            "CREATE SCHEMA ts.raw;
             CREATE TEMP TABLE _rn AS SELECT osm_id FROM s.raw.nodes WHERE lon BETWEEN {w} AND {e} AND lat BETWEEN {s} AND {n};
             CREATE TEMP TABLE _rw AS {} UNION SELECT DISTINCT w.osm_id FROM (SELECT osm_id, unnest(refs) AS r FROM s.raw.ways) w
               SEMI JOIN _rn ON _rn.osm_id = w.r;
             CREATE TABLE ts.raw.ways AS SELECT * FROM s.raw.ways WHERE osm_id IN (SELECT osm_id FROM _rw) ORDER BY rowid;
             CREATE TABLE ts.raw.nodes AS SELECT * FROM s.raw.nodes
               WHERE osm_id IN (SELECT osm_id FROM _rn UNION SELECT unnest(refs) FROM ts.raw.ways) ORDER BY rowid;
             DROP TABLE _rn; DROP TABLE _rw",
            osm.join(" UNION ")
        ))?;
    }
    c.exec("DETACH ts")
}

/// The tables whose rows go with their node, their link, their movement or their crossing; the others are the same in
/// each tile and come from the first one.
fn owner_filter(table: &str, sch: &str) -> Option<String> {
    let link = format!("link_id IN (SELECT link_id FROM t.{sch}.link WHERE from_node_id IN (SELECT node_id FROM _own_n))");
    Some(match table {
        "node" | "signal_controller" | "movement" => "node_id IN (SELECT node_id FROM _own_n)".into(),
        "link" => "from_node_id IN (SELECT node_id FROM _own_n)".into(),
        "lane" | "curb_seg" | "link_along" => link,
        "geometry" => format!("geometry_id IN (SELECT geometry_id FROM t.{sch}.link WHERE from_node_id IN (SELECT node_id FROM _own_n))"),
        "location" => format!("({link}) OR (link_id IS NULL AND ref_node_id IN (SELECT node_id FROM _own_n))"),
        "lane_connector" => format!("mvmt_id IN (SELECT mvmt_id FROM t.{sch}.movement WHERE node_id IN (SELECT node_id FROM _own_n))"),
        "crossing" | "lane_crossing" => "crossing_id IN (SELECT crossing_id FROM _own_x)".into(),
        "lane_check" => format!(
            "split_part(id, '_', 1) IN (SELECT link_id::VARCHAR FROM t.{sch}.link WHERE from_node_id IN (SELECT node_id FROM _own_n))
             OR id IN (SELECT mvmt_id FROM t.{sch}.movement WHERE node_id IN (SELECT node_id FROM _own_n))"
        ),
        _ => return None,
    })
}

/// The tables that are the same in each tile.
const SHARED: [&str; 4] = ["config", "use_definition", "use_group", "zone"];

/// Copy the rows of the GMNS db `path` that tile `t` owns into the db of `c` (the first tile also makes the tables).
fn merge(c: &Ctx, path: &Path, t: Tile, first: bool) -> Result<()> {
    c.exec(&format!("ATTACH '{}' AS t (READ_ONLY)", path.display().to_string().replace('\'', "''")))?;
    let (nx, ny) = tile_sql(t.z, "x_coord", "y_coord");
    let (cx, cy) = tile_sql(t.z, "ST_X(ST_Centroid(geom))", "ST_Y(ST_Centroid(geom))");
    let mut st = c
        .con
        .prepare("SELECT schema_name, table_name FROM duckdb_tables() WHERE database_name = 't' ORDER BY schema_name, table_name")
        .map_err(|e| e.to_string())?;
    let tables: Vec<(String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    for (sch, table) in &tables {
        if first {
            c.exec(&format!("CREATE SCHEMA IF NOT EXISTS {sch}"))?;
        }
        c.exec(&format!(
            "CREATE OR REPLACE TEMP TABLE _own_n AS SELECT node_id FROM t.{sch}.node WHERE {nx} = {} AND {ny} = {}",
            t.x, t.y
        ))?;
        if tables.iter().any(|(s, x)| s == sch && x == "crossing") {
            c.exec(&format!("CREATE OR REPLACE TEMP TABLE _own_x AS SELECT crossing_id FROM t.{sch}.crossing WHERE {cx} = {} AND {cy} = {}", t.x, t.y))?;
        }
        let rows = match owner_filter(table, sch) {
            Some(f) => format!("SELECT * FROM t.{sch}.{table} WHERE {f}"),
            None if SHARED.contains(&table.as_str()) => {
                if !first {
                    continue;
                }
                format!("SELECT * FROM t.{sch}.{table}")
            }
            None => return Err(format!("GMNS by tiles: no owner rule for the table {sch}.{table}")),
        };
        if first {
            c.exec(&format!("CREATE TABLE {sch}.{table} AS {rows}"))?;
        } else {
            c.exec(&format!("INSERT INTO {sch}.{table} BY NAME {rows}"))?;
        }
    }
    c.exec("DROP TABLE IF EXISTS _own_n; DROP TABLE IF EXISTS _own_x; DETACH t")
}

/// Make the GMNS db of `c` (the source attached as `s`) one tile at a time. `build` makes the GMNS db of one tile source.
pub(crate) fn by_tiles(
    c: &Ctx,
    out: &Path,
    tiles: &[Tile],
    present: &[String],
    has_raw: bool,
    o: &GmnsOptions,
    build: impl Fn(&Path, &Path, &GmnsOptions) -> Result<()>,
) -> Result<()> {
    let dir = out.with_extension("tiles");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    struct Gone<'a>(&'a Path);
    impl Drop for Gone<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0);
        }
    }
    let _gone = Gone(&dir);
    let one = GmnsOptions { to_csv: None, combined: false, ..o.clone() };
    for (k, t) in tiles.iter().enumerate() {
        let (src, dst) = (dir.join("source.duckdb"), dir.join("gmns.duckdb"));
        for p in [&src, &dst] {
            let _ = std::fs::remove_file(p);
            let _ = std::fs::remove_file(p.with_extension("duckdb.wal"));
        }
        let t0 = std::time::Instant::now();
        tile_source(c, &src, *t, present, has_raw)?;
        let t1 = std::time::Instant::now();
        build(&src, &dst, &one)?;
        let t2 = std::time::Instant::now();
        merge(c, &dst, *t, k == 0)?;
        eprintln!(
            "GMNS: tile {} ({}/{}) in {:.1} s (source {:.1} s, GMNS {:.1} s, merge {:.1} s)",
            t.name(),
            k + 1,
            tiles.len(),
            t0.elapsed().as_secs_f64(),
            (t1 - t0).as_secs_f64(),
            (t2 - t1).as_secs_f64(),
            t2.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    /// The SQL formula gives the tile of `Tile::of`.
    #[test]
    fn tile_sql_is_tile_of() {
        let con = Connection::open_in_memory().unwrap();
        for (lon, lat) in [(-85.7585, 38.2527), (7.4246, 43.7384), (2.35, 48.85), (-179.9, -60.0)] {
            let (x, y) = tile_sql(9, &lon.to_string(), &lat.to_string());
            let got: (i64, i64) = con.query_row(&format!("SELECT {x}, {y}"), [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            let t = Tile::of(9, lon, lat);
            assert_eq!(got, (t.x as i64, t.y as i64));
        }
    }
}
