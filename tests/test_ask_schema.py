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
import { TABLE_NOTES, mainEdges, roadsBy, startSql, describeTables, roadClasses, aboutText } from %r;
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
process.stdout.write(JSON.stringify(out));
"""


@pytest.fixture(scope="module")
def monaco_names(tmp_path_factory):
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
    con = duckdb.connect(str(out / "monaco.duckdb"), read_only=True)
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
