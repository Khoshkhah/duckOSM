import duckdb
import os

duckosm_db = "data/output/somerset.duckdb"
legacy_edges_csv = "/home/kaveh/projects/h3-routing-platform/tools/osm-importer/data/output/Somerset/Somerset_driving_simplified_edges_with_h3.csv"

con = duckdb.connect(duckosm_db)
con.execute(f"CREATE TABLE IF NOT EXISTS legacy_edges AS SELECT * FROM read_csv_auto('{legacy_edges_csv}')")

def print_sql(query):
    print(con.sql(query).show())

print("=== Length Comparison by Highway Type ===")
con.sql("""
    SELECT 
        COALESCE(d.highway, l.highway) as highway,
        round(COALESCE(d.total_length_km, 0), 2) as duckOSM_km,
        round(COALESCE(l.total_length_km, 0), 2) as legacy_km,
        round(COALESCE(d.total_length_km, 0) - COALESCE(l.total_length_km, 0), 2) as diff_km
    FROM (
        SELECT highway, SUM(length_m)/1000 as total_length_km FROM edges GROUP BY highway
    ) d
    FULL OUTER JOIN (
        SELECT highway, SUM(length)/1000 as total_length_km FROM legacy_edges GROUP BY highway
    ) l ON d.highway = l.highway
    ORDER BY COALESCE(l.total_length_km, 0) DESC
""").show()

# Load spatial extension
con.execute("INSTALL spatial; LOAD spatial;")

print("\n=== Sample Edge Geometry Check (Matched by H3) ===")
con.sql("""
    SELECT 
        l.highway as legacy_hw,
        d.highway as duckosm_hw,
        round(l.length, 2) as legacy_len,
        round(d.length_m, 2) as duckosm_len,
        round(d.length_m - l.length, 2) as diff
    FROM legacy_edges l
    JOIN edges d ON (
        l.from_cell::VARCHAR = d.source_h3 AND 
        l.to_cell::VARCHAR = d.target_h3
    )
    LIMIT 15
""").show()

print("\n=== Baseline: Total Length of ALL Ways in raw data ===")
# We need to construct geometry for raw.ways to calculate length
con.execute("""
    CREATE OR REPLACE TEMP TABLE raw_ways_with_len AS
    WITH way_nodes_coords AS (
        SELECT 
            wn.way_id,
            wn.seq,
            n.lon,
            n.lat
        FROM raw.ways w
        JOIN way_nodes wn ON w.osm_id = wn.way_id
        JOIN raw.nodes n ON wn.node_id = n.osm_id
    ),
    way_geoms AS (
        SELECT 
            way_id,
            'LINESTRING(' || list_aggregate(LIST(lon || ' ' || lat ORDER BY seq), 'string_agg', ', ') || ')' as geom_wkt
        FROM way_nodes_coords
        GROUP BY way_id
    )
    SELECT 
        w.osm_id,
        map_extract(w.tags, 'highway')[1] as highway,
        ST_Length_Spheroid(ST_GeomFromText(geom_wkt)) as length_m
    FROM raw.ways w
    JOIN way_geoms g ON w.osm_id = g.way_id
""")

con.sql("""
    SELECT 
        'Raw OSM (All ways)' as source,
        round(SUM(length_m)/1000, 2) as total_km
    FROM raw_ways_with_len
    UNION ALL
    SELECT 
        'duckOSM (Filtered)' as source,
        round(SUM(length_m)/1000, 2) as total_km
    FROM edges
    WHERE is_reverse = FALSE
    UNION ALL
    SELECT 
        'Legacy (Final)' as source,
        round(SUM(length)/1000, 2) as total_km
    FROM legacy_edges
""").show()

print("\n=== Missing Highway Types? (Raw vs Filtered vs Legacy) ===")
con.sql("""
    SELECT 
        COALESCE(r.highway, l.highway) as highway,
        round(COALESCE(r.km, 0), 2) as raw_km,
        round(COALESCE(d.km, 0), 2) as duck_km,
        round(COALESCE(l.km, 0), 2) as legacy_km
    FROM (
        SELECT highway, SUM(length_m)/1000 as km FROM raw_ways_with_len GROUP BY 1
    ) r
    FULL OUTER JOIN (
        SELECT highway, SUM(length_m)/1000 as km FROM edges WHERE is_reverse = FALSE GROUP BY 1
    ) d ON r.highway = d.highway
    FULL OUTER JOIN (
        SELECT highway, SUM(length)/1000 as km FROM legacy_edges GROUP BY 1
    ) l ON r.highway = l.highway
    ORDER BY COALESCE(l.km, 0) DESC
    LIMIT 20
""").show()

print("\n=== Check for Bidirectional Edges ===")
print("DuckOSM (is_reverse count):")
con.sql("SELECT is_reverse, COUNT(*) FROM edges GROUP BY is_reverse").show()

print("\nLegacy (Check if (from, to) and (to, from) both exist):")
# In legacy data, we check if there are (u, v) and (v, u) pairs
con.sql("""
    SELECT 
        (SELECT COUNT(*) FROM legacy_edges) as total_edges,
        (SELECT COUNT(*) FROM (
            SELECT from_cell, to_cell FROM legacy_edges
            INTERSECT
            SELECT to_cell, from_cell FROM legacy_edges
        )) as bi_count
""").show()
