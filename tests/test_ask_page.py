"""``duckosm ask`` in a browser (docs/design/ask_page.md): the map of a Monaco build with roadstyle's data dock and Ask, the changes of the
fixes file as the table ``extra.edits``, a SQL line with a 64-bit edge id, and no write to the data. Headless Chromium (Playwright); the test
skips itself without Playwright and its browser. Ask itself needs the person's key of an AI service, so the test does not call it."""
import os
import re
import socket
import subprocess
import sys
from pathlib import Path

import pytest

from duckosm import Config, DuckOSM

pw = pytest.importorskip("playwright.sync_api")
ROOT = Path(__file__).resolve().parents[1]
FIXES = """overrides:
  - osm_id: 503462470
    lanes: 3
    changed: '2026-11-06T14:32:05+01:00'
extra:
  tag_edits:
    - type: way
      id: 503462470
      set: {name: Boulevard Test}
      changed: '2026-11-06T14:32:05+01:00'
turn_restrictions:
  - {from_way: 4097656, via_node: 21912099, to_way: 503462470, restriction: no_left_turn, note: why, changed: '2026-11-05T09:00:00+01:00'}
"""


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    out = tmp_path_factory.mktemp("ask")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    cfg.osm_overrides = None
    cfg.output_path = str(out)
    here = os.getcwd()
    os.chdir(out)
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(here)
    (out / "osm_fixes.yaml").write_text(FIXES)
    return out / "monaco.duckdb"


@pytest.fixture(scope="module")
def page_url(monaco):
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    proc = subprocess.Popen([sys.executable, "-c", "from duckosm.cli import main; main()", "ask", str(monaco), "--port", str(port)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    line = ""
    for _ in range(600):
        line = proc.stdout.readline()
        if "http://" in line or proc.poll() is not None:
            break
    m = re.search(r"http://\S+", line)
    if not m:
        proc.kill()
        pytest.fail(f"duckosm ask did not start: {line}")
    yield m.group(0)
    proc.terminate()
    proc.wait(10)


def test_ask_page(monaco, page_url):
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        assert not page.locator("#ask-error").count(), page.inner_text("#ask-error")
        run = """async (sql) => { try { const r = await window.askPage.data.run(sql); return r ?? null; } catch (e) { return {error: String(e.message || e)}; } }"""
        q = """async (sql) => { const r = await fetch('api/sql', {method: 'POST', body: sql}); return r.ok ? await r.json() : {error: await r.text()}; }"""
        # the changes of the file are the table extra.edits, with the time of each
        r = page.evaluate(q, "SELECT kind, type, osm_id, changed IS NOT NULL AS timed, what FROM extra.edits ORDER BY kind, osm_id")
        assert [row[:4] for row in r["rows"]] == [["object", "way", "503462470", True], ["turn", "way", "4097656", True]], r
        assert "lanes 3" in r["rows"][0][4] and "no_left_turn" in r["rows"][1][4], r
        # a 64-bit edge id comes as a text with all its digits
        r = page.evaluate(q, "SELECT edge_id, osm_id FROM driving.edges WHERE osm_id = 503462470 LIMIT 1")
        assert r["columns"][0]["type"] == "BIGINT" and isinstance(r["rows"][0][0], str) and len(r["rows"][0][0]) >= 15, r
        # the data dock runs the line and lights the roads of the result
        res = page.evaluate(run, "SELECT edge_id, name, highway FROM driving.edges WHERE osm_id = 503462470")
        assert not (res or {}).get("error"), res
        # a result with a geometry column is drawn on the map, all of its rows (the table has the first 1000 only): every building and every node
        for sql, minimum in [("SELECT osm_id, name, geom FROM features.buildings", 1200), ("SELECT node_id, geom FROM driving.nodes", 1700)]:
            page.evaluate(run, sql)
            page.wait_for_function(f"window.askPage.shapes() >= {minimum}", timeout=30_000)
            counted = page.evaluate(q, "SELECT count(*) FROM (" + sql + ")")["rows"][0][0]
            assert page.evaluate("window.askPage.shapes()") == int(counted), (sql, counted)
        # the map switch of the data dock (On or Off): the shapes show with it, and a closed dock shows nothing on the map
        page.evaluate(run, "SELECT node_id, geom FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000 && window.askPage.shapesShown()", timeout=30_000)
        vis = "() => window.map.getLayoutProperty('ask-shapes-pt', 'visibility')"
        assert page.evaluate(vis) == "visible"
        page.evaluate("document.querySelector('.rs-dd-light input').click()")
        page.wait_for_function("!window.askPage.shapesShown()", timeout=10_000)
        assert page.evaluate(vis) == "none" and page.evaluate("window.askPage.data.light()") is False
        page.evaluate("document.querySelector('.rs-dd-light input').click()")
        page.wait_for_function("window.askPage.shapesShown()", timeout=10_000)
        assert page.evaluate(vis) == "visible"
        page.evaluate("window.askPage.data.close()")
        page.wait_for_function("!window.askPage.shapesShown()", timeout=10_000)
        page.evaluate("window.askPage.data.open()")
        page.wait_for_function("window.askPage.shapesShown()", timeout=10_000)
        page.evaluate(run, "SELECT edge_id, name FROM driving.edges LIMIT 5")
        page.wait_for_function("window.askPage.shapes() === 0", timeout=30_000)            # a table with no geometry clears the map
        # a write is refused, and the file of the database does not change
        for sql in ["CREATE TABLE main.x AS SELECT 1", "INSERT INTO extra.edits VALUES ('a', 'b', 1, 'c', NULL, NULL)", "DROP TABLE extra.edits"]:
            assert "error" in page.evaluate(q, sql), sql
        assert page.evaluate("fetch('api/build', {method: 'POST'}).then(r => r.json())")["ok"] is False
        assert page.evaluate("fetch('api/table', {method: 'POST', body: '{}'}).then(r => r.status)") == 400
        # roadstyle's modules come from the server
        assert page.evaluate("fetch('roadstyle/ask/ask_panel.js').then(r => r.status)") == 200
        assert page.evaluate("fetch('roadstyle/../Cargo.toml').then(r => r.status)") == 404
        # a click on a building selects it and runs no query: the shapes of the map stay (a new table with no geometry would clear them), and no popup shows
        page.evaluate(run, "SELECT node_id, geom FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000", timeout=30_000)
        shown = page.evaluate("window.askPage.shapes()")
        b = page.evaluate(q, "SELECT b.osm_id, ST_X(ST_PointOnSurface(b.geom)) AS x, ST_Y(ST_PointOnSurface(b.geom)) AS y FROM features.buildings b "
                             "JOIN main.building_faces f ON f.building_id = b.osm_id GROUP BY ALL ORDER BY b.osm_id LIMIT 1")["rows"][0]
        page.evaluate("([x, y]) => { window.map.jumpTo({center: [x, y], zoom: 19, pitch: 0, bearing: 0}); }", [b[1], b[2]])
        page.wait_for_function("window.map.loaded() && !window.map.isMoving()", timeout=30_000)
        pt = page.evaluate("([x, y]) => { const p = window.map.project([x, y]); const r = window.map.getCanvas().getBoundingClientRect(); return [r.left + p.x, r.top + p.y]; }", [b[1], b[2]])
        page.evaluate("window.__picked = null; document.addEventListener('rs:select', (e) => { window.__picked = e.detail.overlay; }, {once: true})")
        page.mouse.click(*pt)
        page.wait_for_function("window.__picked === 'buildings'", timeout=30_000)
        page.wait_for_timeout(500)
        assert page.evaluate("window.askPage.shapes()") == shown, "a click on a building keeps the shapes of the map"
        assert page.locator(".maplibregl-popup").count() == 0, "no popup for a building"
        # the facts of the building are in the card of the data dock: the layer's name as a chip, the name of the building, its id
        page.wait_for_function("document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden", timeout=30_000)
        card = page.inner_text(".rs-dd-card")
        assert "buildings" in card and "Palais princier de Monaco" in card and str(b[0]) in card, card
        # a road: its card beside the table, and no popup
        page.evaluate("() => { const rows = window.rsQuery((p) => Number(p.osm_id) === 503462470); window.rsSelect(rows[0]); }")
        page.wait_for_function("document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden", timeout=30_000)
        assert page.locator(".maplibregl-popup").count() == 0, "no popup for a road"
        # Ask is open in the panel, and buildings are clickable
        assert page.evaluate("window.askPage.ask.isOpen()")
        assert page.evaluate("window.rsGetInteraction('buildings')")["clickable"] is True
        assert not errors, errors
        browser.close()
