//! `duckosm.gis.to_osm_xml`: the network of one mode as an OSM XML file, as OSMnx's `save_graph_xml` writes it. DuckDB
//! writes the file.

use crate::Result;
use std::path::Path;

/// The tags that are columns of the edges with the same name.
const TAGS: [&str; 9] = ["highway", "name", "junction", "layer", "bridge", "tunnel", "service", "surface", "access"];

/// A `<node>` for each node of the roads (`source`, the OSM nodes of `refs`, `target`; where `refs` and the vertices differ, an
/// inner vertex gets a hashed negative id), a `<way>` for each road with the id `edge_id` of its forward edge and the tags. A
/// two-way road is one way. Gives the numbers of nodes and ways.
pub fn to_osm_xml(db: &Path, out: &Path, mode: &str) -> Result<(i64, i64)> {
    if mode.is_empty() || !mode.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{mode:?} is not a mode name"));
    }
    let con = crate::open(db)?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let e = format!("\"{mode}\".edges");
    let attr = |x: &str| format!("replace(replace(replace({x}, '&', '&amp;'), '<', '&lt;'), '\"', '&quot;')");
    let tag = |k: &str, x: &str| {
        format!("CASE WHEN {x} IS NOT NULL THEN '  <tag k=\"{k}\" v=\"' || {} || '\"/>' || chr(10) ELSE '' END", attr(&format!("CAST({x} AS VARCHAR)")))
    };
    let mut tags: Vec<String> = TAGS.iter().map(|k| tag(k, k)).collect();
    tags.extend([
        tag("oneway", "CASE WHEN oneway THEN 'yes' ELSE 'no' END"),
        tag("lanes", "CASE WHEN oneway THEN lanes ELSE lanes + coalesce(twin_lanes, lanes) END"),
        tag("lanes:forward", "CASE WHEN NOT oneway THEN lanes END"),
        tag("lanes:backward", "CASE WHEN NOT oneway THEN coalesce(twin_lanes, lanes) END"),
        tag("maxspeed", "CAST(round(maxspeed_kmh) AS INTEGER)"),
        tag("duckosm:osm_id", "osm_id"),
        tag("duckosm:edge_id", "edge_id"),
    ]);
    let ids = "CASE WHEN len(e.refs) = ST_NPoints(e.geometry) THEN [e.source] || e.refs[2:len(e.refs) - 1] || [e.target] \
               ELSE [e.source] || list_transform(range(2, ST_NPoints(e.geometry)), i -> -(hash(e.edge_id, i) >> 2)::BIGINT) || [e.target] END";
    let roads = format!(
        "SELECT e.*, {ids} AS ids, t.lanes AS twin_lanes FROM {e} e LEFT JOIN {e} t ON t.osm_id = e.osm_id AND \
         t.source = e.target AND t.target = e.source AND t.is_reverse WHERE NOT e.is_reverse"
    );
    let sql = format!(
        "WITH r AS ({roads}),
             p AS (SELECT unnest(ids) AS id, unnest(generate_series(1, len(ids))) AS i, geometry FROM r),
             n AS (SELECT DISTINCT ON (id) id, ST_Y(ST_PointN(geometry, CAST(i AS INTEGER))) AS lat,
                          ST_X(ST_PointN(geometry, CAST(i AS INTEGER))) AS lon FROM p ORDER BY id),
             body AS (
               SELECT 0 AS part, id AS k, '<node id=\"' || id || '\" version=\"1\" lat=\"' || printf('%.7f', lat) ||
                      '\" lon=\"' || printf('%.7f', lon) || '\"/>' AS line FROM n
               UNION ALL
               SELECT 1, edge_id, '<way id=\"' || edge_id || '\" version=\"1\">' || chr(10) ||
                      list_aggr(list_transform(ids, x -> '  <nd ref=\"' || x || '\"/>' || chr(10)), 'string_agg', '') ||
                      {} || '</way>' FROM r)
         SELECT line FROM (
             SELECT -1 AS part, 0 AS k, '<?xml version=\"1.0\" encoding=\"UTF-8\"?>' || chr(10) ||
                    '<osm version=\"0.6\" generator=\"duckOSM\">' AS line
             UNION ALL SELECT part, k, line FROM body
             UNION ALL SELECT 2, 0, '</osm>') ORDER BY part, k",
        tags.join(" || ")
    );
    let path = out.display().to_string().replace('\'', "''");
    let er = |x: duckdb::Error| x.to_string();
    con.execute_batch(&format!("COPY ({sql}) TO '{path}' (FORMAT csv, HEADER false, QUOTE '', ESCAPE '', DELIMITER '\t')")).map_err(er)?;
    con.query_row(&format!("SELECT count(DISTINCT x), count(DISTINCT edge_id) FROM (SELECT unnest(ids) x, edge_id FROM ({roads}))"), [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .map_err(er)
}

/// One row of `features`: `feature_id`, the value of each asked key, all the tags, the geometry (WKB).
pub type Feature = (String, Vec<Option<String>>, Vec<(String, String)>, Vec<u8>);

/// `duckosm.gis.features`: the rows of the QuackOSM GeoParquet file `parquet` that match `tags`, as OSMnx's `tags`: a key
/// with no values matches any value, a key with values matches one of them; the keys are joined with OR, and no keys match
/// all rows. In the order of `feature_id`.
pub fn features(parquet: &Path, tags: &[(String, Option<Vec<String>>)]) -> Result<Vec<Feature>> {
    let con = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    con.execute_batch("INSTALL spatial; LOAD spatial;").map_err(|e| e.to_string())?;
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let cond: Vec<String> = tags
        .iter()
        .map(|(k, vals)| match vals {
            None => format!("tags[{}] IS NOT NULL", q(k)),
            Some(v) => format!("tags[{}] IN ({})", q(k), v.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ")),
        })
        .collect();
    let keys: String = tags.iter().map(|(k, _)| format!(", tags[{}]", q(k))).collect();
    let sql = format!(
        "SELECT feature_id{keys}, map_entries(tags), ST_AsWKB(geometry)::BLOB FROM read_parquet({}) WHERE {} ORDER BY feature_id",
        q(&parquet.display().to_string()),
        if cond.is_empty() { "true".into() } else { cond.join(" OR ") }
    );
    let e = |x: duckdb::Error| x.to_string();
    let mut st = con.prepare(&sql).map_err(e)?;
    let n = tags.len();
    let rows = st
        .query_map([], |r| {
            let vals = (0..n).map(|i| r.get::<_, Option<String>>(1 + i)).collect::<std::result::Result<Vec<_>, _>>()?;
            let pairs = match r.get::<_, duckdb::types::Value>(1 + n)? {
                duckdb::types::Value::List(l) => l
                    .into_iter()
                    .filter_map(|x| match x {
                        duckdb::types::Value::Struct(s) => {
                            let mut it = s.iter().map(|(_, v)| if let duckdb::types::Value::Text(t) = v { t.clone() } else { String::new() });
                            Some((it.next()?, it.next()?))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Ok((r.get(0)?, vals, pairs, r.get(2 + n)?))
        })
        .map_err(e)?;
    rows.collect::<std::result::Result<Vec<Feature>, _>>().map_err(e)
}
