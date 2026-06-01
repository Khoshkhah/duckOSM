import duckdb
import pandas as pd
from shapely import wkt

mtime = 12345
db_path = "/home/kaveh/projects/h3-routing-platform/data/cell_832b9bfffffffff.duckdb"
mode = "driving"
limit = 50000


con = duckdb.connect(db_path, read_only=True)
con.execute("INSTALL spatial; LOAD spatial;")

print(f"Testing fetch for {mode} in {db_path}...")

# Discover available columns in the table
try:
    cols_df = con.execute(f"DESCRIBE {mode}.edges").df()
    available_cols = set(cols_df['column_name'].tolist())
    print(f"Available columns: {available_cols}")
except Exception as e:
    print(f"Error discovering columns for {mode}: {e}")
    exit(1)

# Define desired columns and their fallbacks
select_fields = []

if 'geometry' in available_cols: select_fields.append("ST_AsText(geometry) as wkt_geom")
else: 
    print("NO GEOMETRY COLUMN")
    exit(1)

if 'osm_id' in available_cols: select_fields.append("osm_id")
elif 'id' in available_cols: select_fields.append("id as osm_id")
else: select_fields.append("0 as osm_id")

if 'length_m' in available_cols: select_fields.append("round(length_m, 1) as length_m")
elif 'length' in available_cols: select_fields.append("round(length, 1) as length_m")
else: select_fields.append("0.0 as length_m")

query = f"""
    SELECT 
        {', '.join(select_fields)}
    FROM {mode}.edges
    ORDER BY length_m DESC
    LIMIT {limit}
"""
print(f"Query: {query}")

df = con.execute(query).df()
if df.empty: 
    print("Returned DataFrame is EMPTY")
else:
    print(f"Returned {len(df)} rows")
    print(df.head())
    
    # Check WKT parsing and Parallel Offset
    print("Testing WKT parsing and Parallel Offset...")
    success_count = 0
    fail_count = 0
    multiline_count = 0
    
    for idx, row in df.iterrows():
        try:
            line = wkt.loads(row['wkt_geom'])
            # Simulate visualize.py logic
            # if row['is_reverse']: 
            offset_line = line.parallel_offset(0.00003, 'right', join_style=2)
            
            if offset_line.geom_type == 'MultiLineString':
                multiline_count += 1
                # visualize.py expects simple LineString with .coords
                # attempts to access .coords on MultiLineString will fail
                try:
                    list(offset_line.coords)
                except:
                    # This confirms the bug
                    pass
            else:
                list(offset_line.coords)
                
            success_count += 1
        except Exception as e:
            fail_count += 1
            if fail_count < 5:
                print(f"Failed on index {idx}: {e}")

    print(f"Success: {success_count}, Failed: {fail_count}, MultiLineStrings: {multiline_count}")


# Check metadata
try:
    meta = con.execute("SELECT * FROM main.visualization").fetchall()
    print(f"Metadata: {meta}")
except Exception as e:
    print(f"Metadata check failed: {e}")
