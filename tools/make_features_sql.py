"""Write rust/duckosm-build/sql/features.sql: duckOSM's SQL of the base-map layers (features.*), recorded from
duckOSM's own FeaturesBuilder, so the browser and duckOSM run the same SQL (docs/features.md). Run it again when
duckOSM's layers change.

    /home/kaveh/projects/duckOSM/.venv/bin/python tools/make_features_sql.py [path/to/duckOSM]

The file first loads raw_nodes.parquet, raw_ways.parquet and raw_relations.parquet (duckosm-rust --raw, or the page)
into raw.*, with the column types of duckOSM. One change to duckOSM's SQL: geom.way_line comes from the `line` column of
raw_ways.parquet (each way as WKB, made in Rust), not from geom.way_pt and ST_MakeLine: these two statements were 75 %
of the time (Tartu). The sea (features.ocean) is not in it.
"""
import pathlib
import subprocess
import sys

import duckdb

DUCKOSM = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/home/kaveh/projects/duckOSM")
sys.path.insert(0, str(DUCKOSM / "src"))
from duckosm import __version__  # noqa: E402
from duckosm.features import FeaturesBuilder  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent.parent / "rust/duckosm-build/sql/features.sql"
LOAD = """CREATE SCHEMA IF NOT EXISTS raw;
CREATE OR REPLACE TABLE raw.nodes AS SELECT osm_id, lat, lon, tags FROM read_parquet('raw_nodes.parquet');
CREATE OR REPLACE TABLE raw.ways AS SELECT osm_id, tags, refs FROM read_parquet('raw_ways.parquet');
CREATE OR REPLACE TABLE raw.relations AS
    SELECT osm_id, tags, refs, ref_roles, ref_types::ENUM('node', 'way', 'relation')[] AS ref_types
    FROM read_parquet('raw_relations.parquet');"""


# duckOSM's geom.way_pt and geom.way_line (ensure_foundation) -> the lines that Rust wrote
WAY_LINE = """CREATE OR REPLACE TABLE geom.way_line AS
    SELECT osm_id, tags, (refs[1] = refs[len(refs)]) AS is_closed, ST_GeomFromWKB(line) AS geom
    FROM read_parquet('raw_ways.parquet') WHERE line IS NOT NULL;"""


class Recorder:
    """A connection that keeps each statement (not the counts of the log)."""

    def __init__(self, con):
        self.con, self.sql = con, []

    def execute(self, sql, *args):
        if not sql.lstrip().startswith("SELECT count"):
            self.sql.append(sql.strip().rstrip(";") + ";")
        return self.con.execute(sql, *args)


con = duckdb.connect()
con.execute("""LOAD spatial; CREATE SCHEMA raw;
    CREATE TABLE raw.nodes (osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR));
    CREATE TABLE raw.ways (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[]);
    CREATE TABLE raw.relations (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[], ref_roles VARCHAR[],
                                ref_types ENUM('node', 'way', 'relation')[])""")
rec = Recorder(con)
stats = FeaturesBuilder(rec).run()
starts = [s.split(" AS")[0].split()[-1] for s in rec.sql]
i = starts.index("geom.way_pt")
assert starts[i + 1] == "geom.way_line", f"duckOSM's foundation changed: {starts[:5]}"
rec.sql[i:i + 2] = [WAY_LINE]
if any(v < 0 for v in stats.values()):
    sys.exit(f"a layer failed: {stats}")
commit = subprocess.run(["git", "-C", str(DUCKOSM), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
head = (f"-- duckOSM {__version__} ({commit}): the base-map layers (features.*) from raw.*, recorded from FeaturesBuilder by\n"
        "-- tools/make_features_sql.py. Do not change it by hand. It needs DuckDB's spatial extension.")
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text("\n\n".join([head, LOAD, *rec.sql]) + "\n")
print(f"{OUT}: {len(rec.sql)} statements, layers {', '.join(stats)}")
