"""A fixes file of the duckosm-rust page acts in `duckosm build` as on the page (docs/design/fixes_editor.md).

The file (tests/data/page_fixes/osm_fixes.yaml) has every fix kind: overrides, new_ways, turn_restrictions, extra.tag_edits and
extra.building_faces. page_tables.json has the digests of the tables that the page builds with it (scripts/page_tables.py). The build
here has the same input (data/sample/monaco.osm.pbf in monaco.geojson), the three modes and options.building_faces. Every table of the
page, but the two that only the page makes, must be equal in all its columns but the geometry. Then the tag edits of `raw.*`
(`DuckOSM._raw_way_fixes`) and the walls (`options.building_faces`) are tested too.
"""
import json
from pathlib import Path

import duckdb

from duckosm import Config, DuckOSM
from tests import page_tables as P

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "tests/data/page_fixes"
PAGE_ONLY = {"main.building_entrances", "main.building_info"}     # the page makes them for its panel, from features.buildings and raw.*
PAGE_ONLY_COLUMNS = {"main.boundary": {"geofabrik"}}               # the Geofabrik regions of the page's area: there each column is compared


def test_page_fixes_file_gives_the_tables_of_the_page(tmp_path):
    page = json.loads((DATA / "page_tables.json").read_text())["tables"]
    cfg = Config.from_args(str(ROOT / "data/sample/monaco.osm.pbf"), str(tmp_path), name="monaco",
                           boundary_path=str(ROOT / "data/sample/monaco.geojson"), modes=["driving", "walking", "cycling"],
                           osm_overrides=str(DATA / "osm_fixes.yaml"), building_faces=True)
    cfg.options.sea = None                     # the sea comes from the network (S3): not in a test
    cfg.report.enabled = False
    con = duckdb.connect(str(DuckOSM(cfg).run()), read_only=True)
    con.execute("LOAD spatial")

    def q(sql):
        cur = con.execute(sql)
        return [dict(zip([d[0] for d in cur.description], r)) for r in cur.fetchall()]
    mine = set(P.tables(q))
    bad = []
    for t, want in sorted(page.items()):
        if t in PAGE_ONLY:
            continue
        assert t in mine, f"{t}: the page has the table, duckosm build does not"
        cols = set(want["columns"]) - PAGE_ONLY_COLUMNS.get(t, set())
        missing = cols - set(P.columns(q, t))
        assert not missing, f"{t}: no columns {sorted(missing)}"
        got = P.digest(q, t, sorted(cols))
        if got["n"] != want["n"] or any(got["columns"][c] != want["columns"][c] for c in cols) or (
                t not in PAGE_ONLY_COLUMNS and got["rows"] != want["rows"]):
            bad.append(f"{t}: {got['n']} rows (page {want['n']}), columns {sorted(c for c in cols if got['columns'][c] != want['columns'][c])}")
    assert not bad, "\n".join(bad)
    # the fixes act: the tag edit of a node makes a traffic signal, the wall rule is in main.building_faces
    assert q("SELECT count(*) AS n FROM features.traffic WHERE osm_id = 21912089")[0]["n"] == 1
    assert q("SELECT face_type, edited FROM main.building_faces WHERE building_id = 23715051 AND edited")[0] == {"face_type": "back", "edited": True}
