import duckdb
import os

# Paths
duckosm_db = "data/output/somerset.duckdb"
legacy_dir = "/home/kaveh/projects/h3-routing-platform/tools/osm-importer/data/output/Somerset"

legacy_edges_csv = f"{legacy_dir}/Somerset_driving_simplified_edges_with_h3.csv"
legacy_nodes_csv = f"{legacy_dir}/Somerset_driving_simplified_nodes.csv"
legacy_graph_csv = f"{legacy_dir}/Somerset_driving_edge_graph.csv"

# Connect to DuckDB
con = duckdb.connect(duckosm_db)

# Load legacy data into temp tables
con.execute(f"DROP TABLE IF EXISTS legacy_edges")
con.execute(f"DROP TABLE IF EXISTS legacy_nodes")
con.execute(f"DROP TABLE IF EXISTS legacy_graph")
con.execute(f"CREATE TABLE legacy_edges AS SELECT * FROM read_csv_auto('{legacy_edges_csv}')")
con.execute(f"CREATE TABLE legacy_nodes AS SELECT * FROM read_csv_auto('{legacy_nodes_csv}')")
con.execute(f"CREATE TABLE legacy_graph AS SELECT * FROM read_csv_auto('{legacy_graph_csv}')")

print("=== Comparison Report: duckOSM vs Legacy osm-importer (Somerset) ===\n")

# 1. Feature Counts
print("Counts and Metadata:")
res = con.execute("""
    SELECT 
        'duckOSM' as source,
        (SELECT COUNT(*) FROM nodes) as node_count,
        (SELECT COUNT(*) FROM edges) as edge_count,
        (SELECT COUNT(*) FROM edge_graph) as adj_count,
        round((SELECT SUM(length_m) FROM edges), 2) as total_length_m,
        round((SELECT AVG(cost_s) FROM edges), 2) as avg_cost_s
    UNION ALL
    SELECT 
        'Legacy' as source,
        (SELECT COUNT(*) FROM legacy_nodes) as node_count,
        (SELECT COUNT(*) FROM legacy_edges) as edge_count,
        (SELECT COUNT(*) FROM legacy_graph) as adj_count,
        round((SELECT SUM(length) FROM legacy_edges), 2) as total_length_m,
        round((SELECT AVG(cost) FROM legacy_edges), 2) as avg_cost_s
""").fetchall()
for r in res:
    print(r)

print("\n=== Highway Type Distribution (Edge Count) ===")
res = con.execute("""
    SELECT 
        COALESCE(d.highway, l.highway) as highway,
        COALESCE(d.count, 0) as duckOSM_cnt,
        COALESCE(l.count, 0) as legacy_cnt
    FROM (
        SELECT highway, COUNT(*) as count FROM edges GROUP BY highway
    ) d
    FULL OUTER JOIN (
        SELECT highway, COUNT(*) as count FROM legacy_edges GROUP BY highway
    ) l ON d.highway = l.highway
    ORDER BY COALESCE(d.count, 0) DESC
    LIMIT 15
""").fetchall()
for r in res:
    print(r)

print("\n=== Summary Observations ===")
print("1. Edge counts differ because duckOSM creates 1 edge per Way (Phase 1), while legacy splits ways into multiple segments.")
print("2. Total length difference indicates if road filtering rules (highway types EXCLUDED) match.")
print("3. Adjacency counts differ due to different edge segmentation strategies.")
