//! `duckosm overture-buildings`: add the building attributes of Overture Maps to `features.buildings`.
//!
//! Overture keeps the OSM id of each building that comes from OSM (`sources[].record_id`, for example `w123@4`), and adds
//! attributes from other datasets to it (mostly the heights of Microsoft ML Buildings). The command joins on that id and
//! adds a tag only where the OSM building does not have it: OSM wins. With `overwrite`, the Overture value replaces a
//! different OSM value, and the OSM value stays in `osm:<key>`. Buildings that are only in Overture are not added.

use crate::Result;
use std::path::Path;

const OVERTURE: &str = "s3://overturemaps-us-west-2/release";

/// The result of [`overture_buildings`].
pub struct Summary {
    /// The Overture release that was read, for example `2026-11-18.0`.
    pub release: String,
    /// The buildings of `features.buildings`.
    pub buildings: i64,
    /// The buildings that have an Overture record.
    pub matched: i64,
    /// The buildings that got one or more new tags (not counting `overture:id`).
    pub updated: i64,
    /// The number of new tags of each key, the most frequent first.
    pub tags: Vec<(String, i64)>,
    /// The buildings that got one or more Overture values in place of OSM values (only with `overwrite`).
    pub changed: i64,
    /// The number of changed values of each key, the most frequent first (only with `overwrite`).
    pub changed_tags: Vec<(String, i64)>,
}

/// Add the attributes of Overture Maps (release `release`, or the newest one) to `features.buildings` of `db`, in place.
/// `overwrite`: an Overture value also replaces a different OSM value; the OSM value stays in `osm:<key>`.
pub fn overture_buildings(db: &Path, release: Option<&str>, overwrite: bool) -> Result<Summary> {
    // an in-memory connection with the file attached: the S3 settings stay out of the file
    let con = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    let e = |e: duckdb::Error| e.to_string();
    con.execute_batch("INSTALL httpfs; LOAD httpfs; INSTALL spatial; LOAD spatial; SET s3_region = 'us-west-2'; SET enable_progress_bar = false")
        .map_err(e)?;
    con.execute(&format!("ATTACH '{}' AS d", db.display().to_string().replace('\'', "''")), []).map_err(e)?;
    let has: i64 = con
        .query_row("SELECT count(*) FROM duckdb_tables() WHERE database_name = 'd' AND schema_name = 'features' AND table_name = 'buildings'", [], |r| r.get(0))
        .map_err(e)?;
    if has == 0 {
        return Err(format!("{}: no features.buildings (build with --features)", db.display()));
    }
    let (buildings, x0, y0, x1, y1): (i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>) = con
        .query_row(
            "SELECT count(*), min(ST_XMin(geom)), min(ST_YMin(geom)), max(ST_XMax(geom)), max(ST_YMax(geom)) FROM d.features.buildings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(e)?;
    let (Some(x0), Some(y0), Some(x1), Some(y1)) = (x0, y0, x1, y1) else {
        return Ok(Summary { release: release.unwrap_or_default().into(), buildings, matched: 0, updated: 0, tags: vec![], changed: 0, changed_tags: vec![] });
    };
    let release = match release {
        Some(r) => r.to_string(),
        None => con
            .query_row(
                &format!("SELECT max(regexp_extract(file, 'release/([^/]+)/', 1)) FROM glob('{OVERTURE}/*/theme=buildings/type=building/*')"),
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("Overture Maps not reachable: {e}"))?,
    };
    // the new tags of each OSM building, in the OSM keys; numbers rounded to cm, with no ".0"
    let num = |c: &str| format!("regexp_replace(round({c}, 2)::VARCHAR, '\\.0$', '')");
    con.execute_batch(&format!(
        "CREATE TEMP TABLE ov AS
         SELECT CASE left(rid, 1) WHEN 'w' THEN 'way' WHEN 'r' THEN 'relation' ELSE 'node' END AS osm_type,
                regexp_extract(rid, '^[nwr](\\d+)', 1)::BIGINT AS osm_id, name,
                map_from_entries(list_filter([
                    ('height', {h}), ('min_height', {mh}), ('building:levels', num_floors::VARCHAR),
                    ('building:levels:underground', num_floors_underground::VARCHAR), ('building:min_level', min_floor::VARCHAR),
                    ('roof:shape', roof_shape), ('roof:height', {rh}), ('roof:direction', {rd}),
                    ('roof:orientation', roof_orientation), ('roof:material', roof_material), ('roof:colour', roof_color),
                    ('building:colour', facade_color), ('building:material', facade_material),
                    ('source:height', CASE WHEN height IS NOT NULL THEN height_source END), ('overture:id', id)
                ], t -> t[2] IS NOT NULL)) AS tags
         FROM (SELECT *, names.primary AS name,
                      list_filter(sources, s -> s.dataset = 'OpenStreetMap')[1].record_id AS rid,
                      list_filter(sources, s -> s.property = '/properties/height')[1].dataset AS height_source
               FROM read_parquet('{OVERTURE}/{release}/theme=buildings/type=building/*')
               WHERE bbox.xmin < {x1} AND bbox.xmax > {x0} AND bbox.ymin < {y1} AND bbox.ymax > {y0})
         WHERE rid IS NOT NULL;
         CREATE TEMP TABLE added AS
         SELECT b.osm_type, b.osm_id,
                list_filter(map_keys(ov.tags), k -> NOT list_contains(map_keys(b.tags), k)) AS keys,
                list_filter(map_keys(ov.tags), k -> k != 'overture:id' AND b.tags[k] != ov.tags[k]) AS diff
         FROM d.features.buildings b JOIN ov USING (osm_type, osm_id);
         {update}",
        update = if overwrite {
            // Overture wins; a replaced OSM value stays in osm:<key>
            "UPDATE d.features.buildings b
             SET tags = map_concat(b.tags, map_from_entries(list_transform(a.diff, k -> ('osm:' || k, b.tags[k]))), ov.tags),
                 name = coalesce(ov.name, b.name)
             FROM ov JOIN added a USING (osm_type, osm_id) WHERE b.osm_type = ov.osm_type AND b.osm_id = ov.osm_id;"
        } else {
            "UPDATE d.features.buildings b SET tags = map_concat(ov.tags, b.tags), name = coalesce(b.name, ov.name)
             FROM ov WHERE b.osm_type = ov.osm_type AND b.osm_id = ov.osm_id;"
        },
        h = num("height"),
        mh = num("min_height"),
        rh = num("roof_height"),
        rd = num("roof_direction"),
    ))
    .map_err(|e| format!("Overture Maps {release}: {e}"))?;
    let (matched, updated): (i64, i64) = con
        .query_row("SELECT count(*), count(*) FILTER (WHERE len(list_filter(keys, k -> k != 'overture:id')) > 0) FROM added", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(e)?;
    let mut st = con
        .prepare("SELECT k, count(*) AS n FROM (SELECT unnest(keys) AS k FROM added) WHERE k != 'overture:id' GROUP BY k ORDER BY n DESC, k")
        .map_err(e)?;
    let tags = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    let (mut changed, mut changed_tags) = (0, vec![]);
    if overwrite {
        changed = con.query_row("SELECT count(*) FROM added WHERE len(diff) > 0", [], |r| r.get(0)).map_err(e)?;
        let mut st = con.prepare("SELECT k, count(*) AS n FROM (SELECT unnest(diff) AS k FROM added) GROUP BY k ORDER BY n DESC, k").map_err(e)?;
        changed_tags = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<std::result::Result<_, _>>().map_err(e)?;
    }
    con.execute_batch("CHECKPOINT d").map_err(e)?;
    Ok(Summary { release, buildings, matched, updated, tags, changed, changed_tags })
}
