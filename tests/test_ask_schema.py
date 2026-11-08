"""What Ask knows about the tables (web/ask/schema.js, docs/design/ask_schema.md): a note for every table of a Monaco build, an about text that names a table only when it is there,
and the helpers that both pages (``duckosm ask`` and duckOSM Studio) call. The module has no DOM and no DuckDB, so Node runs it with fake queries. Skips without ``node``."""
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

SCRIPT = """
import { TABLE_NOTES, mainEdges, roadsBy, startSql, describeTables, roadClasses, aboutText, objectKinds, hasTags, askTables, HINTS } from %r;
const names = JSON.parse(process.argv[1]);
const bare = (n) => n.slice(n.lastIndexOf(".") + 1);
const out = {};
out.missing = names.filter((n) => !TABLE_NOTES[bare(n)]);
out.main = [mainEdges(names), mainEdges(["walking.edges", "cycling.edges"]), mainEdges(["main.boundary"])];
out.roadsBy = roadsBy("driving.edges")("SELECT name FROM x", "name");
out.noMain = (() => { try { roadsBy(null)("s", "c"); return "no error"; } catch (e) { return e.message; } })();
out.start = [startSql("driving.edges"), startSql(null)];
const query = async (sql) => sql.includes("information_schema")
  ? { columns: [], rows: [["edge_id", "BIGINT"], ["highway", "VARCHAR"]] }
  : sql.includes("walking") ? Promise.reject(new Error("no table")) : { columns: [], rows: [["residential", 5], [null, 2]] };
const d = await describeTables(names.map((n) => ({ name: n, rows: 1 })), "driving.edges", query);
out.described = d.tables.filter((t) => t.main).map((t) => [t.name, t.columns]);
out.noNote = d.tables.filter((t) => !t.about).map((t) => t.name);
out.classes = await roadClasses(query, ["driving.edges", "walking.edges", "main.boundary"]);
out.about = aboutText({ tables: names, classes: out.classes });
out.aboutNone = aboutText({ tables: ["driving.edges"] });
out.aboutShapes = aboutText({ tables: names, shapes: true });
// the object kinds: the SQL of each (kind, id column, result) pair, for the test to run on the database
const asks = [["node", "node_id", "SELECT node_id FROM driving.nodes LIMIT 5"], ["node", "source", "SELECT source FROM driving.edges LIMIT 5;"],
  ["node", "via_node", "SELECT via_node FROM driving.turn_restrictions LIMIT 5"], ["node", "osm_id", "SELECT osm_id FROM features.traffic LIMIT 5"],
  ["building", "building_id", "SELECT DISTINCT building_id FROM main.building_faces LIMIT 5"], ["building", "osm_id", "SELECT osm_id FROM features.buildings LIMIT 5"],
  ["building", "osm_id", "SELECT osm_id FROM features.traffic LIMIT 5"], ["node", "osm_id", "SELECT osm_id FROM features.buildings LIMIT 5"]];
out.kinds = Object.fromEntries([true, false].map((t) => [t, (() => { const k = objectKinds({ names, hasTags: t }); return { ids: Object.fromEntries(Object.entries(k).map(([n, v]) => [n, v.idColumns])), sql: asks.map(([kind, col, sql]) => k[kind].geometryBy(sql, col)) }; })()]));
out.onlyRoads = Object.keys(objectKinds({ names: ["driving.edges"] }));
out.noNodes = (() => { try { objectKinds({ names: ["features.traffic"] }).node.geometryBy("s", "node_id"); return "no error"; } catch (e) { return e.message; } })();
out.hasTags = [await hasTags(async () => ({ rows: [[1]] })), await hasTags(async () => ({ rows: [] }))];
out.askTables = askTables([{ name: "features_traffic.parquet", mode: "features", table: "traffic", rows: 7, bytes: 1 }, { name: "raw.ways", rows: 1 }, { name: "main.boundary", rows: 2 }, { name: "raw_ways.parquet", mode: "raw", table: "ways", rows: 4 }, { mode: "driving", table: "edges", rows: 3 }]);
out.hints = HINTS.map((h) => [h.table, h.sql, h.tags]);
out.aboutNoTags = aboutText({ tables: names, hasTags: false });
process.stdout.write(JSON.stringify(out));
"""


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    out = tmp_path_factory.mktemp("schema")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    cfg.osm_overrides = None
    cfg.output_path = str(out)
    here = os.getcwd()
    os.chdir(out)                     # the reports of the build stay out of the repository
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(here)
    return out / "monaco.duckdb"


@pytest.fixture(scope="module")
def monaco_names(monaco):
    con = duckdb.connect(str(monaco), read_only=True)
    names = [f"{s}.{t}" for s, t in con.execute("SELECT schema_name, table_name FROM duckdb_tables() WHERE schema_name <> 'raw' ORDER BY 1, 2").fetchall()]
    return names + ["extra.edits", "extra.boundary_h3"]       # the two tables that the page of `duckosm ask` makes


@pytest.fixture(scope="module")
def result(monaco_names):
    module = (ROOT / "web" / "ask" / "schema.js").as_uri()
    done = subprocess.run(["node", "--input-type=module", "-e", SCRIPT % module, json.dumps(monaco_names)], capture_output=True, text=True, timeout=60)
    assert done.returncode == 0, done.stderr
    return json.loads(done.stdout)


def test_every_table_of_a_monaco_build_has_a_note(result):
    assert result["missing"] == [], "add a note for each of these tables to TABLE_NOTES in web/ask/schema.js"
    assert result["noNote"] == []


def test_the_main_table_the_roads_and_the_first_sql(result):
    assert result["main"] == ["driving.edges", "walking.edges", None]
    assert result["roadsBy"] == 'SELECT DISTINCT edge_id FROM driving.edges WHERE "name" IN (SELECT "name" FROM (SELECT name FROM x))'
    assert "no edges" in result["noMain"]
    assert result["start"][0].startswith("SELECT highway, count(*) AS edges") and "FROM driving.edges" in result["start"][0] and result["start"][1] == "SELECT 1"


def test_the_columns_of_the_main_table_and_the_road_classes(result):
    assert result["described"] == [["driving.edges", [{"name": "edge_id", "type": "BIGINT"}, {"name": "highway", "type": "VARCHAR"}]]]
    assert result["classes"] == "driving.edges: residential (5), NULL (2)", "a network that gives an error is left out"


def test_the_about_text_names_a_table_only_when_it_is_there(result):
    full, none = result["about"], result["aboutNone"]
    for name in ["main.building_faces", "extra.edits", "extra.boundary_h3", "driving.turn_restrictions", "edge_graph", "tags['building:levels']"]:
        assert name in full, name
        assert name not in none, f"{name} is not in a file that has only driving.edges"
    assert "driving.edges: residential (5)" in full and "never guess a class" in full and "highway (the road classes)" not in none
    assert "edge_id" in none and "color_roads" in none


def test_the_text_about_shapes_is_for_a_page_that_draws_them(result):
    assert "GEOMETRY COLUMN" in result["aboutShapes"] and "HEXAGONS" in result["aboutShapes"] and "only show roads" in result["aboutShapes"]
    assert "GEOMETRY COLUMN" not in result["about"] and "HEXAGONS" not in result["about"] and "only show roads" not in result["about"]


def test_the_object_kinds_give_the_geometry_and_the_facts_of_a_node_or_a_building(monaco, result):
    """Run the SQL of each kind on the Monaco database: a node, a point of the base map and a building have a geometry, and the facts of the card."""
    con = duckdb.connect(str(monaco), read_only=True)
    con.execute("LOAD spatial")
    assert result["kinds"]["true"]["ids"] == {"node": ["node_id", "source", "target", "via_node", "osm_id"], "building": ["building_id", "osm_id"]}
    assert result["onlyRoads"] == [] and "no nodes" in result["noNodes"]
    assert result["hasTags"] == [True, False]
    for with_tags in ("true", "false"):
        got = []
        for sql in result["kinds"][with_tags]["sql"]:
            cur = con.execute(sql)
            cols, rows = [d[0] for d in cur.description], cur.fetchall()
            got.append((cols, len(rows), all(r[1] is not None for r in rows)))
        node, edge_end, via, traffic, wall, building, wrong_building, wrong_node = got
        assert node[0][:3] == ["id", "geom", "h3_cell"] and node[1] == 5 and node[2], "a node of the graph"
        assert edge_end[1] > 0 and via[1] > 0, "the ends of an edge and the via node of a restriction are nodes"
        assert traffic[0][:4] == ["id", "geom", "kind", "name"] and traffic[1] == 5 and traffic[2], "a point of the base map has its own geometry"
        assert wall[1] == 5 and wall[2] and building[0][:5] == ["id", "geom", "osm_type", "kind", "name"] and building[1] == 5, "a wall is a building by building_id"
        assert wrong_building[1] == 0 and wrong_node[1] == 0, "an osm_id of another kind gives no row"
        assert ("tags" in node[0]) is (with_tags == "true") and ("tags" in building[0]) is (with_tags == "true"), "the tags are left out when the file has none"


def test_ask_sees_the_base_map_tables_and_gets_the_sql_for_the_common_questions(monaco, result):
    """The fault: Studio hid every features.* table, so Ask could not count the traffic signals. askTables keeps them, the text names them and gives SQL to copy."""
    assert result["askTables"] == [{"name": "features.traffic", "rows": 7}, {"name": "main.boundary", "rows": 2}, {"name": "driving.edges", "rows": 3}], \
        "features.* stays, raw.* goes; mode.table wins over a file name, and a plain name still works"
    full, none = result["about"], result["aboutNoTags"]
    for h in result["hints"]:
        assert h[1] in full, f"the hint {h[1]!r} is in the text"
    assert "traffic: traffic_signals or crossing (there are no stop signs)" in full and "never list features.streets" in full
    assert "NO tags column" in none and "tags['building:levels']" not in none and "try_cast" not in none and "features.traffic WHERE kind = 'traffic_signals'" in none, "without tags only the hints that need no tags stay"
    con = duckdb.connect(str(monaco), read_only=True)
    con.execute("LOAD spatial")
    by_table = {h[0]: con.execute(h[1]).fetchall() for h in result["hints"]}       # the SQL of each hint runs on a real database
    # the numbers, with a second query that does not use the hint: 22 signals (the tag highway=traffic_signals), 179 buildings with more than 5 levels
    assert by_table["features.traffic"] == [(22,)] == con.execute("SELECT count(*) FROM features.traffic WHERE tags['highway'] = 'traffic_signals'").fetchall()
    assert by_table["features.buildings"] == [(179,)] == con.execute(
        "SELECT count(*) FROM features.buildings WHERE regexp_matches(tags['building:levels'], '^[0-9]+$') AND CAST(tags['building:levels'] AS INTEGER) > 5").fetchall()
    assert by_table["features.pois"][0] == ("bench", 248) and len(by_table["features.pois"]) > 10
