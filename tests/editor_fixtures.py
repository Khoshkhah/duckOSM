"""The shared kit of the page tests of the editor: the Monaco build, the page server (`duckosm edit`) and a server on a copy of the database whose features tables have no
tags column (an area that Studio restores from files), the helpers that open the page and pick a road, and the constants of the tests. The test files of the editor import it:
`from editor_fixtures import ...` (a fixture imported into a test module is found by pytest)."""
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

pw = pytest.importorskip("playwright.sync_api")
ROOT = Path(__file__).resolve().parents[1]
ROAD = 503462470                      # a road of the Monaco sample (docs/design/fixes_editor.md)
WALL_BUILDING = []                     # the building_id of the building with the most walls (set by the monaco fixture)
BLUE, AMBER = "#2563eb", "#f59e0b"    # the sets fix-sel and fix-edits (ROAD_SETS of fixes_panel.js)
# the colour of the highlight set on each map row of the road (the feature state rs_hl that roadstyle's road layer reads)
_LIT = """(id) => window.rsQuery((p) => Number(p.osm_id) === id).map((row) => { const l = window.map.getLayer("roads-simple") ?? window.map.getLayer("roads-fill");
    return window.map.getFeatureState({source: l.source, sourceLayer: l.sourceLayer, id: row}).rs_hl ?? null; })"""


TURN_OUT, TURN_IN = "#16a34a", "#c026d3"      # the sets fix-turn-out and fix-turn-in
_QUERY = "async (sql) => { const r = await fetch('api/query', {method: 'POST', body: sql}); return await r.json(); }"


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    """A Monaco build of the sample config, with no fixes file."""
    out = tmp_path_factory.mktemp("edit")
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
    db = out / "monaco.duckdb"
    # the building with the most walls, for the wall hover (read now: the page's server has the database open later)
    con = duckdb.connect(str(db), read_only=True)
    WALL_BUILDING.append(con.execute("SELECT building_id FROM main.building_faces GROUP BY building_id ORDER BY count(*) DESC LIMIT 1").fetchone()[0])
    con.close()
    return db


def _free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _serve(db):
    """Start `duckosm edit` on the database: (the process, the URL of the page)."""
    port = _free_port()
    proc = subprocess.Popen([sys.executable, "-c", "from duckosm.cli import main; main()", "edit", str(db), "--port", str(port)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    line = ""
    for _ in range(600):
        line = proc.stdout.readline()
        if "http://" in line or proc.poll() is not None:
            break
    m = re.search(r"http://\S+", line)
    if not m:
        proc.kill()
        pytest.fail(f"duckosm edit did not start: {line}")
    return proc, m.group(0)


@pytest.fixture(scope="module")
def page_url(monaco):
    proc, url = _serve(monaco)
    yield url
    proc.terminate()
    proc.wait(10)


@pytest.fixture(scope="module")
def restored_url(monaco, tmp_path_factory):
    """The shape of an area that Studio restores from its saved files: every table of `features` WITHOUT the column `tags` (the export leaves it out)."""
    d = tmp_path_factory.mktemp("restored")
    db = d / "monaco.duckdb"
    shutil.copy(monaco, db)
    con = duckdb.connect(str(db))
    con.execute("LOAD spatial")
    for (t,) in con.execute("SELECT table_name FROM information_schema.columns WHERE table_schema = 'features' AND column_name = 'tags'").fetchall():
        con.execute(f"CREATE OR REPLACE TABLE features.{t} AS SELECT * EXCLUDE (tags) FROM features.{t}")
    assert not con.execute("SELECT count(*) FROM information_schema.columns WHERE table_schema = 'features' AND column_name = 'tags'").fetchone()[0]
    con.close()
    proc, url = _serve(db)
    yield url
    proc.terminate()
    proc.wait(10)


def _open(browser, url, **ctx):
    page = browser.new_context(**ctx).new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(url)
    page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
    # the map's load event has come: on it the page fits the camera to the area, so a camera move of a test before it is lost (the panel is ready first)
    page.wait_for_function("window.map && window.map._loaded", timeout=60_000)
    return page, errors


def _pick_road(page, osm_id):
    """A click on the road on the map: roadstyle's dashboard picks it (rs:select), and the panel reads its OSM id from the road's row."""
    page.evaluate("""(id) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); const b = window.RS_SIDE.bboxes[rows[0]];
        window.map.jumpTo({center: [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2], zoom: 18}); }""", osm_id)
    page.wait_for_function("window.map.loaded() && !window.map.isMoving()", timeout=30_000)
    page.evaluate("""(id) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); window.rsSelect(rows[0]); }""", osm_id)
    page.wait_for_function(f"document.getElementById('fix-id').value === '{osm_id}' && document.querySelector('#fix-obj .objhead')", timeout=30_000)


def _reset_fixes(url):
    """Start a test with no changes: write an empty fixes file through the page's server (a test module shares one file). Open the page after this."""
    import urllib.request
    origin = url.rstrip("/")
    req = urllib.request.Request(origin + "/api/fixes", data=b"", method="PUT", headers={"Origin": origin, "Content-Type": "text/yaml"})
    urllib.request.urlopen(req, timeout=20).read()


def _open_clean(browser, url, **ctx):
    """_open after _reset_fixes: a page with no changes in it."""
    _reset_fixes(url)
    return _open(browser, url, **ctx)
