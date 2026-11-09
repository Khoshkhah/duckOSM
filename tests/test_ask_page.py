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


def test_off_switch_shows_no_shapes_after_a_reload(monaco, page_url):
    """The map switch of the data dock is kept in the browser, so a page can start Off, and no event `light` comes. Off, the shapes of a result are not drawn."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context()
        ctx.add_init_script("try { localStorage.setItem('rs-data-light:/', '0'); } catch (e) {}")
        page = ctx.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        page.evaluate("window.askPage.data.open()")
        page.evaluate("(s) => window.askPage.data.run(s)", "SELECT node_id, geom FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000", timeout=30_000)
        assert page.evaluate("window.askPage.data.light()") is False
        assert page.evaluate("window.map.getLayoutProperty('ask-shapes-pt', 'visibility')") == "none", "the switch is Off: no shapes on the map"
        page.evaluate("document.querySelector('.rs-dd-light input').click()")
        page.wait_for_function("window.map.getLayoutProperty('ask-shapes-pt', 'visibility') === 'visible'", timeout=10_000)
        assert not errors, errors
        browser.close()


def test_a_row_of_the_table_selects_its_item(monaco, page_url):
    """A row of a result with a geometry column (a node, a building) is drawn on the map in the select look, the map zooms to it, and its facts are in the card of the
    data dock; the close button of the card removes the drawing. A road row is selected by roadstyle, not by the page."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        page.evaluate("window.askPage.data.open()")
        run = "(s) => window.askPage.data.run(s)"
        rows = "#rs-dock tbody tr"
        card_text = "() => (document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden) ? document.querySelector('.rs-dd-card').innerText : ''"
        # a node
        page.evaluate(run, "SELECT node_id, geom FROM driving.nodes WHERE node_id > 0 ORDER BY node_id")
        page.wait_for_function("window.askPage.shapes() > 1000", timeout=30_000)
        page.locator(rows).nth(1).click()
        page.wait_for_function("window.askPage.picked() === 1", timeout=15_000)
        node_id = page.evaluate("document.querySelectorAll('#rs-dock tbody tr')[1].cells[0].innerText.replace(/\\s/g, '')")
        assert node_id in page.evaluate(card_text).replace(" ", "").replace("\u00a0", ""), page.evaluate(card_text)
        assert page.locator(".maplibregl-popup").count() == 0
        page.locator(".rs-dd-rc-x").click()                                   # the close button of the card
        page.wait_for_function("window.askPage.picked() === 0", timeout=15_000)
        # a building, with its name as the title of the card
        page.evaluate(run, "SELECT osm_id, name, geom FROM features.buildings WHERE name IS NOT NULL ORDER BY name")
        page.wait_for_function("document.querySelectorAll('#rs-dock tbody tr').length > 1", timeout=30_000)
        name = page.evaluate("document.querySelectorAll('#rs-dock tbody tr')[0].cells[1].innerText")
        page.locator(rows).nth(0).click()
        page.wait_for_function("window.askPage.picked() === 1", timeout=15_000)
        assert name in page.evaluate(card_text), (name, page.evaluate(card_text))
        # the map switch is for the table only: Off, the shapes of the result go, and the selected item stays on the map, with its card
        page.evaluate("document.querySelector('.rs-dd-light input').click()")
        page.wait_for_function("window.map.getLayoutProperty('ask-shapes-fill', 'visibility') === 'none'", timeout=15_000)
        assert page.evaluate("window.map.getLayoutProperty('ask-pick-fill', 'visibility')") == "visible" and page.evaluate("window.askPage.picked()") == 1
        page.locator(rows).nth(2).click()
        page.wait_for_function("window.askPage.picked() === 1", timeout=15_000)
        assert page.evaluate("window.map.getLayoutProperty('ask-pick-fill', 'visibility')") == "visible", "Off: the selected item is still on the map"
        assert page.evaluate(card_text), "Off: the card still has its facts"
        page.evaluate("document.querySelector('.rs-dd-light input').click()")                  # On again
        # a road row: roadstyle selects the road and draws its card; the page has no drawing of its own
        page.evaluate(run, "SELECT edge_id, name FROM driving.edges ORDER BY edge_id LIMIT 5")
        page.wait_for_function("document.querySelectorAll('#rs-dock tbody tr').length === 5", timeout=30_000)
        page.locator(rows).nth(0).click()
        page.wait_for_function("window.askPage.picked() === 0", timeout=15_000)
        assert not errors, errors
        browser.close()


def test_an_h3_row_is_selected_on_the_map_and_the_opacity_tool(monaco, page_url):
    """A table of H3 cells (a hex text, or a BIGINT) is drawn as hexagons by roadstyle's data dock (the kind h3cell of web/ask/views.js, h3-js makes the shapes). A click on a
    row selects the hexagon and the card tells what the cell is and how many nodes it holds. The card of a node has a button that draws its cell. The tool set_shapes of Ask
    changes the opacity of the objects and brings the default back."""
    drawn = "(s) => (window.map.getSource(s) ? window.map.getSource(s).serialize().data.features.length : 0)"
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        page.evaluate("window.askPage.data.open()")
        run, rows = "(s) => window.askPage.data.run(s)", "#rs-dock tbody tr"
        q = "async (sql) => { const r = await fetch('api/sql', {method: 'POST', body: sql}); return r.ok ? await r.json() : {error: await r.text()}; }"
        card_text = "() => (document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden) ? document.querySelector('.rs-dd-card').textContent.replace(/\\s/g, '') : ''"
        # a hex text
        page.evaluate(run, "SELECT h3_cell FROM extra.boundary_h3 WHERE resolution = 9 ORDER BY h3_cell")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 20", timeout=30_000)
        assert page.evaluate("window.askPage.shapes()") == 0, "the page draws nothing for a result that a kind owns"
        cell = page.evaluate("document.querySelectorAll('#rs-dock tbody tr')[1].cells[0].innerText.trim()")
        page.locator(rows).nth(1).click()
        page.wait_for_function(f"({drawn})('rs-dock-kinds-sel') === 1", timeout=15_000)
        page.wait_for_function("() => document.querySelector('.rs-dd-card').textContent.includes('parent cell')", timeout=15_000)
        text = page.evaluate(card_text)
        assert cell in text and "resolution9" in text and "area" in text and "edgelength" in text, text
        # a BIGINT cell of the nodes: the card counts the nodes of the cell
        page.evaluate(run, "SELECT DISTINCT h3_cell FROM driving.nodes ORDER BY h3_cell")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 3", timeout=30_000)
        page.locator(rows).nth(0).click()
        hexed = page.evaluate(q, "SELECT lower(to_hex(h3_cell)), count(*) FROM driving.nodes WHERE h3_cell = (SELECT min(h3_cell) FROM driving.nodes) GROUP BY 1")["rows"][0]
        page.wait_for_function(f"({drawn})('rs-dock-kinds-sel') === 1", timeout=15_000)
        page.wait_for_function("() => document.querySelector('.rs-dd-card').textContent.includes('Inside this cell')", timeout=15_000)
        text = page.evaluate(card_text)
        assert hexed[0] in text and ("nodes" + hexed[1]) in text, (hexed, text)
        # the card of a node has a button for its cell; the cell is drawn with the node
        page.evaluate(run, "SELECT node_id FROM driving.nodes WHERE node_id > 0 ORDER BY node_id LIMIT 5")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') >= 5", timeout=30_000)
        page.locator(rows).nth(0).click()
        page.wait_for_function("() => !!document.querySelector('.rs-dd-rc-act button')", timeout=15_000)
        page.locator(".rs-dd-rc-act button").click()
        page.wait_for_function(f"({drawn})('rs-dock-kinds-sel') >= 2", timeout=15_000)
        # the opacity of the objects (the tool of the page)
        paint = "() => [window.map.getPaintProperty('rs-dock-kinds-fill', 'fill-opacity'), window.map.getPaintProperty('rs-dock-kinds-line', 'line-opacity')]"
        page.evaluate(run, "SELECT h3_cell FROM extra.boundary_h3 WHERE resolution = 9")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 20", timeout=30_000)
        assert page.evaluate(paint) == [1, 1]
        done = page.evaluate("window.askPage.shapeTools[0].run({opacity: 0.15})")
        assert "0.15" in done["done"] and page.evaluate(paint) == [0.15, 0.15]
        page.evaluate(run, "SELECT h3_cell FROM extra.boundary_h3 WHERE resolution = 8")            # a new result keeps the opacity
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 3", timeout=30_000)
        page.wait_for_function("() => window.map.getPaintProperty('rs-dock-kinds-fill', 'fill-opacity') === 0.15", timeout=15_000)
        assert page.evaluate("window.askPage.shapeTools[0].run({reset: true})")["done"].startswith("The shapes have the default look")
        assert page.evaluate(paint) == [1, 1]
        assert "opacity must be" in page.evaluate("window.askPage.shapeTools[0].run({opacity: 3}).then(() => 'no error', (e) => e.message)")
        assert not errors, errors
        browser.close()


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
        # H3: the page makes the table extra.boundary_h3 (h3-js), and a result with a column h3_cell is drawn as hexagons
        whole = page.evaluate(q, "SELECT max(resolution), count(*) FROM extra.boundary_h3 WHERE whole_boundary")["rows"][0]
        # every resolution from 0 to the smallest one has one cell that holds the whole boundary (Monaco: the smallest is resolution 6)
        assert 4 <= int(whole[0]) <= 8 and int(whole[1]) == int(whole[0]) + 1, whole
        page.evaluate("window.askPage.data.open()")
        drawn = "(s) => (window.map.getSource(s) ? window.map.getSource(s).serialize().data.features.length : 0)"
        page.evaluate(run, "SELECT h3_cell FROM extra.boundary_h3 WHERE resolution = 10")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 50", timeout=30_000)
        page.evaluate(run, "SELECT DISTINCT h3_cell FROM driving.nodes")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 5", timeout=30_000)
        # the colour of a shape: the walls by their face_type (the colours of the editor), and any shape by a column `color`
        page.evaluate("window.askPage.data.open()")
        page.evaluate(run, "SELECT building_id, face_type, geometry FROM main.building_faces")
        page.wait_for_function("window.askPage.shapes() > 5000", timeout=30_000)
        types = page.evaluate("[...new Set(window.askPage.shapeProps().map((p) => p.face_type))].sort()")
        assert types == ["back", "court", "front", "party", "side"], types
        assert page.evaluate("window.map.getPaintProperty('ask-shapes-line', 'line-color')")[0] == "coalesce"
        page.evaluate(run, "SELECT node_id, geom, CASE WHEN node_id % 2 = 0 THEN '#ff0000' ELSE 'not a colour' END AS color FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000 && window.askPage.shapeProps().some((p) => p.color)", timeout=30_000)       # the shapes of the new result, not the walls before
        colours = page.evaluate("[...new Set(window.askPage.shapeProps().map((p) => p.color ?? 'none'))].sort()")
        assert colours == ["#ff0000", "none"], colours                       # a colour that CSS does not know is left out
        # the examples of Ask: roadstyle's general groups come first, then the groups of the page, each with its heading and none twice; all the questions are there
        ex = page.evaluate("document.querySelector('.rs-ask-ex').textContent")      # textContent: the headings may be in capitals by CSS, which innerText returns
        titles = [g["title"] for g in page.evaluate("window.askPage.questions")]
        assert "Know the data" in ex and ex.index("Know the data") < ex.index(titles[0]), "the general groups come first"
        assert all(ex.count(t) == 1 for t in titles), "no heading twice"
        asked = page.evaluate("[...document.querySelectorAll('.rs-ask-ex .rs-ask-q')].map((b) => b.textContent)")
        mine = [q for g in page.evaluate("window.askPage.questions") for q in g["items"]]
        assert all(q in asked for q in mine) and len(asked) > len(mine), "every question of the page is a chip, after the general ones"
        # Ask is told the real road classes (it guessed them, found no roads, and had no step left): the values of highway are in its instructions
        classes = page.evaluate(q, "SELECT DISTINCT highway FROM driving.edges WHERE highway IS NOT NULL")["rows"]
        about = page.evaluate("window.askPage.about")
        assert classes and all(f"{c[0]} (" in about for c in classes), "every road class of the driving network is in the instructions"
        # the map switch of the data dock (On or Off): the shapes show with it, and a closed dock shows nothing on the map
        page.evaluate("window.askPage.data.open()")
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
        # the roads of a result are lit by roadstyle on their own shape (edge_id): no line of the page over them, also when the table has their geometry column
        page.evaluate(run, "SELECT edge_id, name, geometry FROM driving.edges")
        page.wait_for_function("window.askPage.shapes() === 0", timeout=30_000)
        page.evaluate(run, "SELECT node_id, geom FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000", timeout=30_000)
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


def test_a_result_of_ids_has_its_objects_on_the_map_and_a_card(monaco, page_url):
    """web/ask/views.js gives roadstyle's data dock the kinds of the tables (docs/design/table_views.md): a result with ids and no geometry column (the junction of a
    turn restriction, the nodes of a way, a bus route) is drawn by the dock itself, a click on a row selects the object and fills the card. A result with a geometry
    column keeps the page's own drawing."""
    drawn = "(s) => (window.map.getSource(s) ? window.map.getSource(s).serialize().data.features.length : 0)"
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        page.evaluate("window.askPage.data.open()")
        run, rows = "(s) => window.askPage.data.run(s)", "#rs-dock tbody tr"
        card = "() => (document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden) ? document.querySelector('.rs-dd-card').innerText : ''"
        # the junction of a turn restriction: a node of the graph
        page.evaluate(run, "SELECT restriction_id, via_node FROM driving.turn_restrictions")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 0", timeout=30_000)
        assert page.evaluate("window.askPage.shapes()") == 0, "the page draws nothing for a result that a kind owns"
        page.locator(rows).nth(0).click()
        page.wait_for_function(f"({drawn})('rs-dock-kinds-sel') === 1", timeout=15_000)
        via = page.evaluate("document.querySelectorAll('#rs-dock tbody tr')[0].cells[1].innerText.replace(/\\s/g, '')")
        assert via in page.evaluate(card).replace(" ", "").replace(" ", ""), page.evaluate(card)
        # the nodes of a way: most are not junctions, so the point comes from raw.nodes
        page.evaluate(run, "SELECT way_id, node_id FROM driving.way_nodes LIMIT 300")
        page.wait_for_function(f"({drawn})('rs-dock-kinds') > 250", timeout=30_000)
        # a result with a geometry column keeps the page's own drawing, and the kind does not draw it again
        page.evaluate(run, "SELECT node_id, geom FROM driving.nodes")
        page.wait_for_function("window.askPage.shapes() > 1000", timeout=30_000)
        assert page.evaluate(f"({drawn})('rs-dock-kinds')") == 0, "a result with a geometry column is not drawn twice"
        assert not errors, errors
        browser.close()


def test_a_turn_row_draws_its_roads_and_has_a_card(monaco, page_url):
    """A row of edge_graph or turn_restrictions (two roads and the junction) is one object of three parts: roadstyle draws all of them, a click selects the object, and the card
    has its facts from the table (docs/design/table_views.md, roadstyle step 3: parts with roles and a composite key)."""
    drawn = "(s) => (window.map.getSource(s) ? window.map.getSource(s).serialize().data.features.length : 0)"
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("console", lambda m: errors.append(m.text) if m.type == "error" and "ERR_" not in m.text and "Failed to load resource" not in m.text else None)
        page.goto(page_url)
        page.wait_for_function("window.askPage", timeout=60_000)
        page.evaluate("window.askPage.data.open()")
        run, rows = "(s) => window.askPage.data.run(s)", "#rs-dock tbody tr"
        card = "() => (document.querySelector('.rs-dd-card') && !document.querySelector('.rs-dd-card').hidden) ? document.querySelector('.rs-dd-card').innerText : ''"
        for sql, label in [("SELECT from_edge, to_edge FROM driving.edge_graph LIMIT 20", "Turn"),
                           ("SELECT restriction_id, from_edge_id, via_node, to_edge_id FROM driving.turn_restrictions", "Turn restriction")]:
            page.evaluate(run, sql)
            page.wait_for_function(f"({drawn})('rs-dock-kinds') >= 3", timeout=30_000)          # at least one turn: its three parts
            page.locator(rows).nth(0).click()
            page.wait_for_function(f"({drawn})('rs-dock-kinds-sel') === 3", timeout=15_000)       # from, via, to
            assert label.lower() in page.evaluate(card).lower(), (label, page.evaluate(card))
        assert not errors, errors
        browser.close()
