//! `features-check` (duckOSM `features/check.py`): the OSM polygons of `raw.*` that no `features.*` table holds, grouped by
//! their main tag (docs/design/missing_polygons.md). The SQL and the text of the report are those of duckOSM.

/// The tag that names a group, in this order; a group with none of them is `other: <its keys>`.
const KEYS: [&str; 26] = [
    "building", "place", "area:highway", "highway", "man_made", "aeroway", "landuse", "natural", "leisure", "amenity", "tourism",
    "historic", "public_transport", "railway", "waterway", "shop", "office", "boundary", "power", "barrier", "building:part",
    "craft", "healthcare", "sport", "emergency", "military",
];

/// (reason, a SQL condition on the MAP column `tags`): the groups that the features build leaves out on purpose.
const LEFT_OUT: [(&str, &str); 10] = [
    ("a zone or a name, not ground (the name is in place_labels)", "tags['place'] IS NOT NULL AND tags['place'] <> 'square'"),
    ("an administrative or postal boundary: a line, not ground", "tags['boundary'] IS NOT NULL"),
    ("the coastline: the sea comes from the layer ocean", "tags['natural'] = 'coastline'"),
    ("a part of a building: planned for a layer building_parts (phase 2)", "tags['building:part'] IS NOT NULL"),
    (
        "gone: a building, a place or a boundary that is demolished, disused or abandoned",
        "tags['building'] = 'no' OR len(list_filter(map_keys(tags), lambda x: x LIKE 'demolished:%' OR x LIKE 'disused:%' OR x LIKE 'abandoned:%')) > 0",
    ),
    (
        "only an address, a name or a source: no kind of ground",
        "len(list_filter(map_keys(tags), lambda x: NOT (x LIKE 'addr:%' OR x IN ('type', 'name', 'source', 'note', 'fixme', 'wikidata', 'wikipedia')))) = 0",
    ),
    (
        "a mark of an event (a race: pit lane, animals) or a sport tag without a pitch",
        "tags['line'] IS NOT NULL OR tags['attraction'] IS NOT NULL OR tags['animal'] IS NOT NULL OR tags['sport'] IS NOT NULL",
    ),
    ("a club or an organisation drawn as an area: not ground", "tags['club'] IS NOT NULL"),
    (
        "indoor mapping (rooms, walls, levels): not ground",
        "tags['indoor'] IS NOT NULL AND tags['shop'] IS NULL AND tags['amenity'] IS NULL AND tags['office'] IS NULL \
         AND tags['tourism'] IS NULL AND tags['historic'] IS NULL",
    ),
    (
        "street furniture and small areas (power, barriers, parking spaces, shelters, seating, fountains, bins, …)",
        "tags['power'] IS NOT NULL OR tags['barrier'] IS NOT NULL OR tags['amenity'] IN ('motorcycle_parking','parking_space',\
         'shelter','fountain','bench','waste_basket','bicycle_parking','toilets','drinking_water','taxi','parking_entrance',\
         'recycling','waste_disposal') \
         OR tags['leisure'] IN ('outdoor_seating','fitness_station','picnic_table','bleachers','dance','horse_riding') \
         OR tags['tourism'] IN ('artwork','picnic_site','viewpoint','attraction','information') OR tags['emergency'] IS NOT NULL",
    ),
];

/// A group of polygons that no feature table holds. `area_m2` is the area of the closed ways (the relations are not
/// assembled); `reason` is None for a possible miss.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub group: String,
    pub ways: i64,
    pub relations: i64,
    pub area_m2: f64,
    pub reason: Option<String>,
}

fn label_sql(prefix: &str) -> String {
    let parts: Vec<String> = KEYS.iter().map(|k| format!("WHEN {prefix}tags['{k}'] IS NOT NULL THEN '{k}=' || {prefix}tags['{k}']")).collect();
    format!("CASE {} ELSE 'other: ' || array_to_string(list_sort(map_keys({prefix}tags)), ',') END", parts.join(" "))
}

fn reason_sql(prefix: &str) -> String {
    let parts: Vec<String> = LEFT_OUT
        .iter()
        .map(|(reason, pred)| format!("WHEN {} THEN '{}'", pred.replace("tags[", &format!("{prefix}tags[")), reason.replace('\'', "''")))
        .collect();
    format!("CASE {} END", parts.join(" "))
}

/// The groups of `db` (a file with `raw.*` and `features.*`), the possible misses first, then by area and count.
#[cfg(feature = "duckdb")]
pub fn missing_polygons(db: &std::path::Path) -> Result<Vec<Group>, String> {
    use duckosm_duckdb::duckdb::{AccessMode, Config, Connection};
    let err = |e: duckosm_duckdb::duckdb::Error| e.to_string();
    let cfg = Config::default().access_mode(AccessMode::ReadOnly).map_err(err)?;
    let con = Connection::open_with_flags(db, cfg).map_err(|e| format!("{}: {e}", db.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(err)?;
    let mut st = con.prepare("SELECT table_name FROM duckdb_tables() WHERE schema_name = 'features'").map_err(err)?;
    let tables: Vec<String> = st.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
    if tables.is_empty() {
        return Err("this database has no features.* tables: run the features build first".into());
    }
    let fids = tables.iter().map(|t| format!("SELECT osm_type, osm_id FROM features.{t}")).collect::<Vec<_>>().join(" UNION ALL ");
    con.execute_batch(&format!(
        "CREATE OR REPLACE TEMP VIEW fids AS {fids};
         CREATE OR REPLACE TEMP VIEW rings AS
         SELECT DISTINCT ref_id AS way_id FROM (
             SELECT UNNEST(refs) AS ref_id, UNNEST(ref_types) AS ref_type
             FROM raw.relations WHERE osm_id IN (SELECT osm_id FROM fids WHERE osm_type = 'relation')) WHERE ref_type = 'way';
         CREATE OR REPLACE TEMP TABLE miss_ways AS
         SELECT osm_id, tags, refs FROM raw.ways
         WHERE len(refs) >= 4 AND refs[1] = refs[len(refs)] AND len(map_keys(tags)) > 0
           AND osm_id NOT IN (SELECT osm_id FROM fids WHERE osm_type = 'way') AND osm_id NOT IN (SELECT way_id FROM rings);"
    ))
    .map_err(|e| format!("{e} (the database needs raw.* and features.*: a clipped database has no raw.*)"))?;
    let ways = format!(
        "WITH pts AS (SELECT osm_id, UNNEST(refs) AS nid, generate_subscripts(refs, 1) AS i FROM miss_ways),
         geo AS (SELECT p.osm_id, ST_Area_Spheroid(ST_FlipCoordinates(ST_MakePolygon(ST_MakeLine(list(ST_Point(n.lon, n.lat) ORDER BY p.i))))) AS a
                 FROM pts p JOIN raw.nodes n ON n.osm_id = p.nid GROUP BY 1 HAVING count(*) >= 4)
         SELECT {} AS grp, {} AS reason, count(*) AS ways, 0 AS relations, round(sum(coalesce(g.a, 0))) AS area_m2
         FROM miss_ways m LEFT JOIN geo g ON g.osm_id = m.osm_id GROUP BY 1, 2",
        label_sql("m."),
        reason_sql("m.")
    );
    let rels = format!(
        "SELECT {} AS grp, {} AS reason, 0 AS ways, count(*) AS relations, 0 AS area_m2
         FROM raw.relations r
         WHERE r.tags['type'] IN ('multipolygon', 'boundary') AND len(map_keys(r.tags)) > 1
           AND r.osm_id NOT IN (SELECT osm_id FROM fids WHERE osm_type = 'relation') GROUP BY 1, 2",
        label_sql("r."),
        reason_sql("r.")
    );
    let mut out: Vec<Group> = Vec::new();
    for sql in [ways, rels] {
        let mut st = con.prepare(&sql).map_err(err)?;
        let rows = st
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<f64>>(4)?))
            })
            .map_err(err)?;
        for row in rows {
            let (grp, reason, w, rel, area) = row.map_err(err)?;
            match out.iter_mut().find(|g| g.group == grp) {
                Some(g) => {
                    g.ways += w;
                    g.relations += rel;
                    g.area_m2 += area.unwrap_or(0.0);
                }
                None => out.push(Group { group: grp, ways: w, relations: rel, area_m2: area.unwrap_or(0.0), reason }),
            }
        }
    }
    out.sort_by(|a, b| {
        (a.reason.is_some(), -a.area_m2, -(a.ways + a.relations))
            .partial_cmp(&(b.reason.is_some(), -b.area_m2, -(b.ways + b.relations)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// The report as text (duckOSM `print_report`): the possible misses first, then the groups left out on purpose.
pub fn report(groups: &[Group], limit: usize) -> String {
    if groups.is_empty() {
        return "every tagged polygon of raw.* is in a features table\n".into();
    }
    let open = groups.iter().filter(|g| g.reason.is_none()).count();
    let mut s = format!(
        "{open} group(s) of polygons in no features table, with no reason (a possible miss); {} group(s) left out on purpose.\n\n",
        groups.len() - open
    );
    s += &format!("{:58} {:>6} {:>9} {:>10}  reason\n", "group", "ways", "relations", "area m²");
    for g in groups.iter().take(limit) {
        let name: String = g.group.chars().take(58).collect();
        s += &format!(
            "{name:58} {:6} {:9} {:10.0}  {}\n",
            g.ways,
            g.relations,
            g.area_m2,
            g.reason.as_deref().unwrap_or("-- NOT EXPLAINED --")
        );
    }
    if groups.len() > limit {
        s += &format!("... and {} more (--limit)\n", groups.len() - limit);
    }
    s
}
