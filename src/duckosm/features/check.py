"""``duckosm features-check``: the OSM polygons of ``raw.*`` that no ``features.*`` table holds.

It makes a miss visible (docs/design/missing_polygons.md). It lists

- the closed ways with tags that are in no feature table, and are not a ring of a relation that a feature table holds, and
- the area relations (``type`` = ``multipolygon`` / ``boundary``) with tags that are in no feature table,

grouped by their main tag. Some groups are left out on purpose (``LEFT_OUT``: zones, boundaries, street furniture, building parts,
…): the report names the reason. A group with no reason is a possible miss: add a rule to a layer, or to ``LEFT_OUT``.
"""

import logging

logger = logging.getLogger("duckosm")

# the tag that names a group, in this order; a group with none of them is "other: <its keys>"
KEYS = ["building", "place", "area:highway", "highway", "man_made", "aeroway", "landuse", "natural", "leisure", "amenity", "tourism",
        "historic", "public_transport", "railway", "waterway", "shop", "office", "boundary", "power", "barrier", "building:part",
        "craft", "healthcare", "sport", "emergency", "military"]

# (reason, SQL predicate on the MAP column ``tags``): groups that the features build leaves out on purpose
LEFT_OUT = [
    ("a zone or a name, not ground (the name is in place_labels)",
     "tags['place'] IS NOT NULL AND tags['place'] <> 'square'"),
    ("an administrative or postal boundary: a line, not ground", "tags['boundary'] IS NOT NULL"),
    ("the coastline: the sea comes from the layer ocean", "tags['natural'] = 'coastline'"),
    ("a part of a building: planned for a layer building_parts (phase 2)", "tags['building:part'] IS NOT NULL"),
    ("gone: a building, a place or a boundary that is demolished, disused or abandoned",
     "tags['building'] = 'no' OR len(list_filter(map_keys(tags), x -> x LIKE 'demolished:%' OR x LIKE 'disused:%' OR x LIKE 'abandoned:%')) > 0"),
    ("only an address, a name or a source: no kind of ground",
     "len(list_filter(map_keys(tags), x -> NOT (x LIKE 'addr:%' OR x IN ('type', 'name', 'source', 'note', 'fixme', 'wikidata', 'wikipedia')))) = 0"),
    ("a mark of an event (a race: pit lane, animals) or a sport tag without a pitch",
     "tags['line'] IS NOT NULL OR tags['attraction'] IS NOT NULL OR tags['animal'] IS NOT NULL OR tags['sport'] IS NOT NULL"),
    ("a club or an organisation drawn as an area: not ground", "tags['club'] IS NOT NULL"),
    ("indoor mapping (rooms, walls, levels): not ground", "tags['indoor'] IS NOT NULL"),
    ("street furniture and small areas (power, barriers, parking spaces, shelters, seating, fountains, bins, …)",
     "tags['power'] IS NOT NULL OR tags['barrier'] IS NOT NULL OR tags['amenity'] IN ('motorcycle_parking','parking_space',"
     "'shelter','fountain','bench','waste_basket','bicycle_parking','toilets','drinking_water','taxi','parking_entrance',"
     "'recycling','waste_disposal') "
     "OR tags['leisure'] IN ('outdoor_seating','fitness_station','picnic_table','bleachers','dance','horse_riding') "
     "OR tags['tourism'] IN ('artwork','picnic_site','viewpoint','attraction','information') OR tags['emergency'] IS NOT NULL"),
]


def _label_sql(prefix: str = "") -> str:
    parts = " ".join(f"WHEN {prefix}tags['{k}'] IS NOT NULL THEN '{k}=' || {prefix}tags['{k}']" for k in KEYS)
    return (f"CASE {parts} ELSE 'other: ' || array_to_string(list_sort(map_keys({prefix}tags)), ',') END")


def _reason_sql(prefix: str = "") -> str:
    parts = " ".join(f"WHEN {pred.replace('tags[', prefix + 'tags[')} THEN '{reason.replace(chr(39), chr(39) * 2)}'"
                     for reason, pred in LEFT_OUT)
    return f"CASE {parts} END"


def missing_polygons(con) -> list[dict]:
    """The groups of polygons in ``raw.*`` that no ``features.*`` table holds: ``{group, ways, relations, area_m2, reason}``.
    ``area_m2`` is the area of the closed ways (the relations are not assembled); ``reason`` is None for a possible miss."""
    con.execute("INSTALL spatial; LOAD spatial;")
    tables = [r[0] for r in con.execute("SELECT table_name FROM duckdb_tables() WHERE schema_name = 'features'").fetchall()]
    if not tables:
        raise ValueError("this database has no features.* tables: run the features build first")
    con.execute("CREATE OR REPLACE TEMP VIEW fids AS " + " UNION ALL ".join(f"SELECT osm_type, osm_id FROM features.{t}" for t in tables))
    # ways that are rings of a relation which a feature table holds: they are in the table through the relation
    con.execute("""
        CREATE OR REPLACE TEMP VIEW rings AS
        SELECT DISTINCT ref_id AS way_id FROM (
            SELECT UNNEST(refs) AS ref_id, UNNEST(ref_types) AS ref_type
            FROM raw.relations WHERE osm_id IN (SELECT osm_id FROM fids WHERE osm_type = 'relation')) WHERE ref_type = 'way'""")
    con.execute(f"""
        CREATE OR REPLACE TEMP TABLE miss_ways AS
        SELECT osm_id, tags, refs FROM raw.ways
        WHERE len(refs) >= 4 AND refs[1] = refs[len(refs)] AND len(map_keys(tags)) > 0
          AND osm_id NOT IN (SELECT osm_id FROM fids WHERE osm_type = 'way') AND osm_id NOT IN (SELECT way_id FROM rings)""")
    rows = con.execute(f"""
        WITH pts AS (SELECT osm_id, UNNEST(refs) AS nid, generate_subscripts(refs, 1) AS i FROM miss_ways),
        geo AS (SELECT p.osm_id, ST_Area_Spheroid(ST_FlipCoordinates(ST_MakePolygon(ST_MakeLine(list(ST_Point(n.lon, n.lat) ORDER BY p.i))))) AS a
                FROM pts p JOIN raw.nodes n ON n.osm_id = p.nid GROUP BY 1 HAVING count(*) >= 4)
        SELECT {_label_sql('m.')} AS grp, {_reason_sql('m.')} AS reason, count(*) AS ways, 0 AS relations, round(sum(coalesce(g.a, 0))) AS area_m2
        FROM miss_ways m LEFT JOIN geo g ON g.osm_id = m.osm_id GROUP BY 1, 2""").fetchall()
    rel = con.execute(f"""
        SELECT {_label_sql('r.')} AS grp, {_reason_sql('r.')} AS reason, 0 AS ways, count(*) AS relations, 0 AS area_m2
        FROM raw.relations r
        WHERE r.tags['type'] IN ('multipolygon', 'boundary') AND len(map_keys(r.tags)) > 1
          AND r.osm_id NOT IN (SELECT osm_id FROM fids WHERE osm_type = 'relation') GROUP BY 1, 2""").fetchall()
    out = {}
    for grp, reason, ways, relations, area in rows + rel:
        g = out.setdefault(grp, {"group": grp, "ways": 0, "relations": 0, "area_m2": 0.0, "reason": reason})
        g["ways"] += ways
        g["relations"] += relations
        g["area_m2"] += area or 0
    return sorted(out.values(), key=lambda g: (g["reason"] is not None, -(g["area_m2"]), -g["ways"] - g["relations"]))


def print_report(groups: list[dict], limit: int = 40) -> None:
    """The report as text: the possible misses first, then the groups left out on purpose."""
    if not groups:
        print("every tagged polygon of raw.* is in a features table")
        return
    open_ = [g for g in groups if g["reason"] is None]
    print(f"{len(open_)} group(s) of polygons in no features table, with no reason (a possible miss); "
          f"{len(groups) - len(open_)} group(s) left out on purpose.\n")
    print(f"{'group':58s} {'ways':>6s} {'relations':>9s} {'area m²':>10s}  reason")
    for g in groups[:limit]:
        print(f"{g['group'][:58]:58s} {g['ways']:6d} {g['relations']:9d} {g['area_m2']:10.0f}  {g['reason'] or '-- NOT EXPLAINED --'}")
    if len(groups) > limit:
        print(f"... and {len(groups) - limit} more (--limit)")
