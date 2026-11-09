"""Table views (web/ask/views.js, docs/design/table_views.md): every table of a Monaco build has a status, and the SQL of every kind runs on the database and gives the objects that a
second query gives. The module has no DOM and no DuckDB, so Node writes the SQL and DuckDB runs it. Skips without ``node``.
A table that has no entry in ``TABLES`` of views.js fails the first test, so a new table cannot be forgotten."""
import json
import os
import shutil
import subprocess
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")

# (kind key, id column, SQL of a result, exact: the kind must give one object for each distinct id of the result)
ASKS = [
    ("node", "node_id", "SELECT node_id FROM driving.nodes LIMIT 5", True),
    ("node", "source", "SELECT source FROM driving.edges LIMIT 5", True),
    ("node", "node_id", "SELECT node_id FROM driving.way_nodes LIMIT 5", True),
    ("node", "node_id", "SELECT node_id FROM mm.transfers LIMIT 5", True),
    ("node", "osm_id", "SELECT osm_id FROM features.traffic WHERE osm_type = 'node' LIMIT 5", True),
    ("building", "building_id", "SELECT DISTINCT building_id FROM main.building_faces LIMIT 5", True),
    ("building", "building_id", "SELECT building_id FROM main.building_entrances WHERE building_id IS NOT NULL LIMIT 5", False),
    ("building", "osm_id", "SELECT osm_id FROM features.buildings WHERE osm_type = 'way' LIMIT 5", True),
    ("way", "osm_id", "SELECT DISTINCT osm_id FROM driving.edges LIMIT 5", True),
    ("way", "way_id", "SELECT DISTINCT way_id FROM driving.way_nodes LIMIT 5", False),
    ("way", "osm_id", "SELECT osm_id FROM driving.ways LIMIT 5", False),
    ("route", "osm_id", "SELECT osm_id FROM bus.routes LIMIT 5", False),
    ("route", "route_id", "SELECT DISTINCT route_id FROM bus.route_edges LIMIT 5", False),
    ("boundary", "osm_id", "SELECT osm_id FROM main.boundary", True),
    ("cell", "h3_id", "SELECT h3_id FROM main.boundary_cells", True),
    *[(f"feature_{t}", "osm_id", f"SELECT osm_id FROM features.{t} LIMIT 5", False) for t in
      ("land", "paved", "place_labels", "pois", "public_transport", "sites", "streets", "water_lines", "water_polygons")],
]

SCRIPT = """
import { viewKinds, coverage } from %r;
const names = JSON.parse(process.argv[1]), asks = JSON.parse(process.argv[2]);
const out = { coverage: coverage(names), keys: {}, sql: [] };
for (const t of [true, false]) {
  const k = viewKinds({ names, hasTags: t });
  out.keys[t] = Object.fromEntries(Object.entries(k).map(([n, v]) => [n, v.idColumns]));
  if (t) out.sql = asks.map(([key, col, sql]) => (k[key] ? k[key].geometryBy(sql, col) : null));
}
process.stdout.write(JSON.stringify(out));
"""


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    out = tmp_path_factory.mktemp("views")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    cfg.osm_overrides = None
    cfg.output_path = str(out)
    cfg.options.boundary_cells, cfg.options.boundary_cell_resolutions = True, [6, 8]
    cfg.multimodal.enabled = True
    here = os.getcwd()
    os.chdir(out)                     # the reports of the build stay out of the repository
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(here)
    con = duckdb.connect(str(out / "monaco.duckdb"), read_only=True)
    con.execute("LOAD spatial")
    return con


@pytest.fixture(scope="module")
def names(monaco):
    rows = monaco.execute("SELECT schema_name, table_name FROM duckdb_tables() WHERE schema_name <> 'raw' UNION ALL SELECT schema_name, view_name FROM duckdb_views() WHERE NOT internal AND schema_name = 'mm' ORDER BY 1, 2").fetchall()
    return [f"{s}.{t}" for s, t in rows] + ["raw.nodes", "extra.edits", "extra.boundary_h3"]       # raw.nodes is kept in this build; the last two are made by the page of `duckosm ask`


@pytest.fixture(scope="module")
def result(names):
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    done = subprocess.run(["node", "--input-type=module", "-e", SCRIPT % module, json.dumps(names), json.dumps(ASKS)], capture_output=True, text=True, timeout=60)
    assert done.returncode == 0, done.stderr
    return json.loads(done.stdout)


def test_every_table_has_a_status(result):
    unknown = [n for n, s in result["coverage"].items() if s == "unknown"]
    assert not unknown, f"add these tables to TABLES in web/ask/views.js: {unknown}"


def test_a_kind_status_names_a_kind(result):
    keys = set(result["keys"]["true"])
    for name, status in result["coverage"].items():
        if not status.startswith("kind:"):
            continue
        kind, table = status[5:], name.split(".")[-1]
        assert kind in keys or any(k.startswith(f"{kind}_{table}") for k in keys) or any(k.startswith(f"{kind}_") for k in keys), f"{name} says {status}, but viewKinds has no such kind"


def test_the_sql_of_every_kind_runs(monaco, result):
    for (key, column, sql, exact), view in zip(ASKS, result["sql"]):
        assert view, f"viewKinds has no kind {key}"
        got = monaco.execute(view)
        columns = [d[0] for d in got.description]
        rows = got.fetchall()
        assert columns[:2] == ["id", "geom"], (key, columns)
        assert rows, f"{key} by {column} gave no object for: {sql}"
        for r in rows:
            assert json.loads(r[1])["type"], (key, r[0])        # geom is GeoJSON text
        wanted = len({r[0] for r in monaco.execute(sql).fetchall()})
        assert len(rows) <= wanted, (key, column, len(rows), wanted)
        if exact:
            assert len(rows) == wanted, f"{key} by {column}: {len(rows)} objects for {wanted} ids"


def test_tags_are_left_out_when_the_file_has_none(monaco, names):
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    script = "import { viewKinds } from %r; const k = viewKinds({ names: JSON.parse(process.argv[1]), hasTags: false }); process.stdout.write(JSON.stringify([k.building.geometryBy('SELECT 1', 'osm_id'), k.feature_pois.geometryBy('SELECT 1', 'osm_id')]));" % module
    done = subprocess.run(["node", "--input-type=module", "-e", script, json.dumps(names)], capture_output=True, text=True, timeout=30)
    assert done.returncode == 0, done.stderr
    assert not any("tags" in sql for sql in json.loads(done.stdout))
