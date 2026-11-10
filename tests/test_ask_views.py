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
    # a turn is one object of the parts from, via and to; the key is two columns
    ("turn", ["from_edge", "to_edge"], "SELECT from_edge, to_edge FROM driving.edge_graph LIMIT 5", True),
    ("turn", ["from_edge", "to_edge"], "SELECT from_edge, to_edge FROM walking.edge_graph LIMIT 5", True),
    ("restriction", ["from_edge_id", "to_edge_id"], "SELECT from_edge_id, to_edge_id FROM driving.turn_restrictions", True),
    # these tables are empty on Monaco, so the result is made from edge_graph: the SQL of the kind must still run and draw
    ("path", ["from_edge", "to_edge"], "SELECT from_edge, to_edge, [from_edge] AS via_edges, 'no_u_turn' AS restriction_type, 'car' AS applies_to, NULL AS condition FROM driving.edge_graph LIMIT 3", True),
    ("permission", ["from_edge", "to_edge"], "SELECT from_edge, to_edge, true AS allowed, 'bus' AS vehicles, 'taxi' AS except_vehicles, NULL AS condition FROM driving.edge_graph LIMIT 3", True),
    ("merged", "new_edge_id", "SELECT edge_id AS new_edge_id FROM driving.edges LIMIT 3", None),     # driving.edge_id_map is empty on Monaco: the SQL must run, no rows are asked for
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
        if exact is None:
            continue                                            # the SQL ran; the table has no rows to find
        assert rows, f"{key} by {column} gave no object for: {sql}"
        for r in rows:
            geom = json.loads(r[1])
            if isinstance(geom, list):                          # the parts of a turn: a road in, the junction or the roads between, a road out
                roles = [p["role"] for p in geom]
                assert "from" in roles and "to" in roles and "via" in roles and all(p["geom"]["type"] for p in geom), (key, r[0], roles)
            else:
                assert geom["type"], (key, r[0])                # geom is GeoJSON text
        got_ids = monaco.execute(sql).fetchall()
        wanted = len({tuple(r[:2]) for r in got_ids if None not in r[:2]} if isinstance(column, list) else {r[0] for r in got_ids})
        assert len(rows) <= wanted, (key, column, len(rows), wanted)
        if exact:
            assert len(rows) == wanted, f"{key} by {column}: {len(rows)} objects for {wanted} ids"


def test_tags_are_left_out_when_the_file_has_none(monaco, names):
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    script = "import { viewKinds } from %r; const k = viewKinds({ names: JSON.parse(process.argv[1]), hasTags: false }); process.stdout.write(JSON.stringify([k.building.geometryBy('SELECT 1', 'osm_id'), k.feature_pois.geometryBy('SELECT 1', 'osm_id')]));" % module
    done = subprocess.run(["node", "--input-type=module", "-e", script, json.dumps(names)], capture_output=True, text=True, timeout=30)
    assert done.returncode == 0, done.stderr
    assert not any("tags" in sql for sql in json.loads(done.stdout))


def test_a_gmns_movement_is_a_turn():
    """The movements of a GMNS file: the link in, the node and the link out, one object. Skips without the local GMNS file of Södermalm."""
    path = ROOT / "data" / "db" / "sodermalm_gmns.duckdb"
    if not path.exists():
        pytest.skip("data/db/sodermalm_gmns.duckdb is not built")
    con = duckdb.connect(str(path), read_only=True)
    con.execute("LOAD spatial")
    names = [f"{s}.{t}" for s, t in con.execute("SELECT schema_name, table_name FROM duckdb_tables() ORDER BY 1, 2").fetchall()]
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    script = "import { viewKinds } from %r; const k = viewKinds({ names: JSON.parse(process.argv[1]) }).movement_gmns_driving; process.stdout.write(k.geometryBy('SELECT ib_link_id, ob_link_id FROM gmns_driving.movement LIMIT 5', ['ib_link_id', 'ob_link_id']));" % module
    done = subprocess.run(["node", "--input-type=module", "-e", script, json.dumps(names)], capture_output=True, text=True, timeout=30)
    assert done.returncode == 0, done.stderr
    rows = con.execute(done.stdout).fetchall()
    wanted = len({tuple(r) for r in con.execute("SELECT ib_link_id, ob_link_id FROM gmns_driving.movement LIMIT 5").fetchall()})
    assert len(rows) == wanted and rows, (len(rows), wanted)
    assert {p["role"] for p in json.loads(rows[0][1])} == {"from", "via", "to"}


def test_a_turn_card_needs_only_the_columns_the_table_has():
    """A file built before `uses` has an edge_graph with no `uses` column: the turn kind must still draw its turns, and its card has the columns that exist (cost). The rows keep
    the order of the result, so the first rows of the table are the first objects (the dock cuts the objects at 20 000)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges AS SELECT * FROM (VALUES (1, ST_GeomFromText('LINESTRING(0 0, 1 0)')), (2, ST_GeomFromText('LINESTRING(1 0, 1 1)')), (3, ST_GeomFromText('LINESTRING(1 0, 2 0)'))) t(edge_id, geometry)")
    con.execute("CREATE TABLE driving.edge_graph AS SELECT * FROM (VALUES (3, 1, 1, 7.0), (1, 2, 2, 5.5), (1, 3, 3, 4.5)) t(from_edge, to_edge, via_edge, cost)")
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    script = "import { viewKinds } from %r; const k = viewKinds({ names: ['driving.edges', 'driving.edge_graph'] }).turn; process.stdout.write(k.geometryBy('SELECT * FROM driving.edge_graph', ['from_edge', 'to_edge']));" % module
    done = subprocess.run(["node", "--input-type=module", "-e", script], capture_output=True, text=True, timeout=30)
    assert done.returncode == 0, done.stderr
    got = con.execute(done.stdout)
    assert [d[0] for d in got.description] == ["id", "geom", "cost"], got.description
    rows = got.fetchall()
    assert [r[0] for r in rows] == ["3|1", "1|2", "1|3"], rows           # the order of the table
    assert [r[2] for r in rows] == [7.0, 5.5, 4.5]
    assert {p["role"] for p in json.loads(rows[0][1])} == {"from", "via", "to"}


def test_the_kinds_work_on_the_tables_of_the_rust_engine():
    """duckOSM Studio's tables come from the Rust engine: `nodes` has node_id and geom and no h3_cell. The node kind must draw them (it failed with a binder error). The same pair of
    edges can be in two networks with another cost, so a turn card has no facts of its own then (the section Row has them)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving; CREATE SCHEMA walking")
    con.execute("CREATE TABLE driving.nodes AS SELECT * FROM (VALUES (10, ST_Point(0, 0)), (11, ST_Point(1, 0))) t(node_id, geom)")
    for mode, cost in (("driving", 5.3), ("walking", 31.9)):
        con.execute(f"CREATE TABLE {mode}.edges AS SELECT * FROM (VALUES (1, ST_GeomFromText('LINESTRING(0 0, 1 0)')), (2, ST_GeomFromText('LINESTRING(1 0, 1 1)'))) t(edge_id, geometry)")
        con.execute(f"CREATE TABLE {mode}.edge_graph AS SELECT 1 AS from_edge, 2 AS to_edge, 2 AS via_edge, {cost}::FLOAT AS cost")
    module = (ROOT / "web" / "ask" / "views.js").as_uri()
    names = ["driving.nodes", "driving.edges", "driving.edge_graph", "walking.edges", "walking.edge_graph"]
    script = ("import { viewKinds } from %r; const k = viewKinds({ names: JSON.parse(process.argv[1]) });"
              "process.stdout.write(JSON.stringify([k.node.geometryBy('SELECT 10 AS node_id', 'node_id'), k.turn.geometryBy('SELECT * FROM driving.edge_graph', ['from_edge', 'to_edge'])]));") % module
    done = subprocess.run(["node", "--input-type=module", "-e", script, json.dumps(names)], capture_output=True, text=True, timeout=30)
    assert done.returncode == 0, done.stderr
    node_sql, turn_sql = json.loads(done.stdout)
    rows = con.execute(node_sql).fetchall()
    assert len(rows) == 1 and rows[0][0] == 10 and json.loads(rows[0][1])["type"] == "Point", rows
    got = con.execute(turn_sql)
    assert [d[0] for d in got.description] == ["id", "geom"], got.description        # no cost or uses: two networks have the pair
