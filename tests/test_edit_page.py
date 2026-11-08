"""``duckosm edit`` in a browser (docs/design/edit_page.md): the page of a Monaco build, the roads in roadstyle's highlight sets, a change of a
road saved in osm_fixes.yaml, the box "This road is in your changes" when the road is picked again, and Build again with the file. Headless
Chromium (Playwright); the test skips itself without Playwright and its browser. The map's base layers come from the internet."""
import os
import re
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


@pytest.fixture(scope="module")
def page_url(monaco):
    port = _free_port()
    proc = subprocess.Popen([sys.executable, "-c", "from duckosm.cli import main; main()", "edit", str(monaco), "--port", str(port)],
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
    yield m.group(0)
    proc.terminate()
    proc.wait(10)


def _open(browser, url, **ctx):
    page = browser.new_context(**ctx).new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(url)
    page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
    return page, errors


def _pick_road(page, osm_id):
    """A click on the road on the map: roadstyle's dashboard picks it (rs:select), and the panel reads its OSM id from the road's row."""
    page.evaluate("""(id) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); const b = window.RS_SIDE.bboxes[rows[0]];
        window.map.jumpTo({center: [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2], zoom: 18}); }""", osm_id)
    page.wait_for_function("window.map.loaded() && !window.map.isMoving()", timeout=30_000)
    page.evaluate("""(id) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); window.rsSelect(rows[0]); }""", osm_id)
    page.wait_for_function(f"document.getElementById('fix-id').value === '{osm_id}' && document.querySelector('#fix-obj .objhead')", timeout=30_000)


def test_edit_save_box_and_build(monaco, page_url):
    fixes = monaco.with_name("osm_fixes.yaml")
    assert not fixes.exists()
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        # the dashboard's own panel and toolbox are there, with the editor in the panel
        assert page.locator("#sb .sb-top").is_visible() and page.locator("#fix-tabs").is_visible()
        _pick_road(page, ROAD)
        # the picked road is blue on the map's own road rows (the set fix-sel), and the editor's own line layer has nothing of it
        page.wait_for_function(f"({_LIT})({ROAD}).every((c) => c === '{BLUE}')", timeout=10_000)
        assert page.evaluate("window.fixesPanel.drawn()")["sel"] == 0
        page.fill("#fixf-lanes", "3")
        page.click("#fix-save")
        for _ in range(50):
            if fixes.exists() and "lanes: 3" in fixes.read_text():
                break
            time.sleep(0.1)
        text = fixes.read_text()
        assert f"osm_id: {ROAD}" in text and "lanes: 3" in text, text
        assert re.search(r"changed: '\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d'", text), "each record has its time"
        # the road again: the box first, and the form waits
        page.click("#fix-unselect")
        # an edited road is amber when it is not picked (the set fix-edits)
        page.wait_for_function(f"({_LIT})({ROAD}).every((c) => c === '{AMBER}')", timeout=10_000)
        _pick_road(page, ROAD)
        box = page.locator("#fix-inchg")
        box.wait_for(state="visible")
        assert "This road is in your changes" in box.inner_text() and "lanes 3" in box.inner_text()
        assert not page.locator("#fix-form").is_visible()
        page.click("#fix-inchg [data-ask=edit]")
        assert page.locator("#fix-form").is_visible() and page.input_value("#fixf-lanes") == "3"
        # Build again: the database is built again with the file, and the page loads again
        page.click("#fix-tab-changes")
        assert page.is_enabled("#fix-apply"), page.inner_text("#fix-apply-why")
        with page.expect_navigation(timeout=600_000):
            page.click("#fix-apply")
        page.wait_for_function("window.fixesPanel", timeout=60_000)
        assert page.evaluate("window.fixesPanel.count()") == 1, "the rules come back from the file"
        assert not errors, errors
        browser.close()
    con = duckdb.connect(str(monaco), read_only=True)
    lanes = {r[0] for r in con.execute(f"SELECT lanes FROM driving.edges WHERE osm_id = {ROAD}").fetchall()}
    assert lanes == {3}, lanes
    con.close()


def test_hover_is_on_the_objects_of_the_tab(monaco, page_url):
    """The hover of roadstyle is on the objects of the tab (rs.holdHover): a road under the mouse has the pointer in the Road and Turn tabs, and
    not in the Node, New road and Changes tabs. A click on a road still gives its pick in the Road tab."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        _pick_road(page, ROAD)
        page.evaluate("fixesPanel.tab('road')")
        cursor = "() => window.map.getCanvas().style.cursor"
        spot = None
        for dy in range(-60, 61, 6):                           # a point of the road under the mouse: the road is drawn through the middle of the map
            for dx in range(-60, 61, 6):
                x, y = 760 + dx, 450 + dy
                page.mouse.move(x, y)
                page.wait_for_timeout(40)
                if page.evaluate(cursor) == "pointer":
                    spot = (x, y)
                    break
            if spot:
                break
        assert spot, "no road under the mouse in the Road tab"
        for tab, pointer in [("node", False), ("new", False), ("changes", False), ("turns", True), ("road", True)]:
            page.evaluate(f"fixesPanel.tab('{tab}')")
            page.mouse.move(5, 5)
            page.mouse.move(*spot)
            page.wait_for_timeout(200)
            assert (page.evaluate(cursor) == "pointer") is pointer, (tab, page.evaluate(cursor))
        assert not errors, errors
        browser.close()


def test_add_a_file(monaco, page_url, tmp_path):
    """Add a file (docs/design/fixes_editor_time_merge.md): the changes of another file go into the changes. The road of the first test is
    in the two files, and its change here is newer, so it stays; a node with no time is added and gets the time of now. Undo takes it back."""
    fixes = monaco.with_name("osm_fixes.yaml")
    other = tmp_path / "other.yaml"
    other.write_text(f"overrides:\n  - {{osm_id: {ROAD}, lanes: 2, changed: '2020-01-01T10:00:00+01:00'}}\n"
                     "extra:\n  tag_edits:\n    - {type: node, id: 21912099, set: {highway: stop}}\n")
    bad = tmp_path / "bad.yaml"
    bad.write_text("modes: [driving]\n")
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        page.click("#fix-tab-changes")
        before = page.evaluate("window.fixesPanel.count()")
        page.set_input_files("#fix-add-file", str(bad))
        page.wait_for_function("document.getElementById('fix-status').textContent.includes('is not a fixes file')", timeout=10_000)
        assert page.evaluate("window.fixesPanel.count()") == before, "a bad file changes nothing"
        page.set_input_files("#fix-add-file", str(other))
        page.wait_for_function("document.getElementById('fix-status').textContent.includes('from other.yaml')", timeout=10_000)
        msg = page.inner_text("#fix-status")
        assert "Added 1 change" in msg and "1 object was in both files: the newer change stays" in msg, msg
        assert page.evaluate("window.fixesPanel.count()") == before + 1
        assert page.locator("#fix-list .chgtime").count() == before + 1, "each change shows its time"
        for _ in range(50):
            if "id: 21912099" in fixes.read_text():
                break
            time.sleep(0.1)
        text = fixes.read_text()
        assert "highway: stop" in text and "lanes: 3" in text and "lanes: 2" not in text, text
        assert text.count("changed: '") == text.count("- type:") + text.count("- osm_id:"), text
        page.click("#fix-undo")
        assert page.evaluate("window.fixesPanel.count()") == before
        assert not errors, errors
        browser.close()


def test_phone(monaco, page_url):
    """The page on a phone: the dashboard's sheet has the editor, and nothing is wider than the screen."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url, viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
        assert page.evaluate("document.documentElement.classList.contains('rs-phone')")
        page.locator("#fix-tabs").wait_for(state="visible")
        assert page.evaluate("document.documentElement.scrollWidth") <= 390
        # the tab row of roadstyle's frame is in the screen (on a phone it is one row that scrolls sideways)
        right = page.evaluate("document.getElementById('fix-tabs').getBoundingClientRect().right")
        assert right <= 390, right
        assert not errors, errors
        browser.close()


def test_host_without_road_rows(monaco, page_url):
    """A host that gives `roadRows: false`: the picked road is drawn by the editor's own line layer, not by a highlight set."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context()
        # the page has its script inline (rust/duckosm-edit/src/page.rs): the page with `roadRows: false` in the editor's ctx
        ctx.route(page_url, lambda route: route.fulfill(response=(r := route.fetch()), body=r.text().replace("status, areaKey:", "status, roadRows: false, areaKey:")))
        page = ctx.new_page()
        page.goto(page_url)
        page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
        _pick_road(page, ROAD)
        page.wait_for_function("window.fixesPanel.drawn().sel > 0", timeout=10_000)
        assert page.evaluate("window.fixesPanel.drawn()")["lit"]["fix-sel"] == 0
        assert page.evaluate(f"({_LIT})({ROAD})") == [None] * 2
        browser.close()


def test_the_turn_tab_draws_all_the_roads_at_the_junction(monaco, page_url):
    """The Turn tab: the From road is blue, the roads that leave the junction are green, the roads that only arrive are a third colour, and the legend says so. A road of
    another network at the junction is named in the panel. Every highlight set has opacity 1, so the map has the colour of the legend in the dark theme too."""
    via, frm = 25193390, 4227273
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        ids = lambda sql: [int(r["id"]) for r in page.evaluate(_QUERY, sql)]
        leave = ids(f"SELECT DISTINCT osm_id AS id FROM driving.edges WHERE source = {via} AND osm_id != {frm}")
        only_in = ids(f"SELECT DISTINCT osm_id AS id FROM driving.edges WHERE target = {via} AND osm_id != {frm} "
                      f"AND osm_id NOT IN (SELECT osm_id FROM driving.edges WHERE source = {via})")
        assert leave and only_in, "the junction of the test has roads that leave and roads that only arrive"
        page.evaluate("window.__hl = []; const o = window.rsHighlight; window.rsHighlight = (ids, l) => { if (l && l.set) window.__hl.push(l); return o(ids, l); }")
        page.evaluate("fixesPanel.tab('turns')")
        page.fill("#fix-from", str(frm))
        page.click("#fix-turn-find")
        page.wait_for_function("document.querySelectorAll('#fix-via option').length > 0", timeout=30_000)
        page.evaluate("(v) => { const s = document.getElementById('fix-via'); s.value = v; s.dispatchEvent(new Event('input', {bubbles: true})); s.dispatchEvent(new Event('change', {bubbles: true})); }", str(via))
        for i, c in ((only_in[0], TURN_IN), (leave[0], TURN_OUT), (frm, BLUE)):      # the three sets are drawn one after the other: wait for each
            page.wait_for_function(f"({_LIT})({i}).some((c) => c === '{c}')", timeout=20_000)
        for i in leave:
            assert set(page.evaluate(f"({_LIT})({i})")) == {TURN_OUT}, ("leaves", i)
        for i in only_in:
            assert set(page.evaluate(f"({_LIT})({i})")) == {TURN_IN}, ("only arrives", i)
        assert set(page.evaluate(f"({_LIT})({frm})")) == {BLUE}, "the From road"
        assert page.evaluate("window.__hl.length") > 0 and all(l.get("opacity") == 1 for l in page.evaluate("window.__hl")), "every set has opacity 1"
        assert "Only into it" in page.inner_text("#fix-turn-legend")
        assert "not in the driving network" in page.inner_text("#fix-out"), "a road of another network at the junction is named"
        assert not errors, errors
        browser.close()


def test_the_turn_tab_shows_every_junction_of_the_road_and_a_click_on_a_dot_chooses_it(monaco, page_url):
    """After a road is picked, each of its junctions is a white dot on the map, the same junctions as the Junction box lists. A click on a dot chooses that junction (the box and the
    list of turns follow) and does not pick the road under the dot again."""
    frm = 8352246                                   # a long road with 7 junctions in the Monaco sample
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        page.evaluate("fixesPanel.tab('turns')")
        page.fill("#fix-from", str(frm))
        page.click("#fix-turn-find")
        page.wait_for_function("document.querySelectorAll('#fix-via option').length > 1", timeout=30_000)
        listed = page.evaluate("[...document.querySelectorAll('#fix-via option')].map((o) => o.value)")
        page.wait_for_function(f"fixesPanel.drawn().turnJs === {len(listed)}", timeout=20_000)
        dots = """() => { const m = window.map; return m.queryRenderedFeatures({layers: ['fix-turn-js-pt']})
            .map((f) => { const q = m.project(f.geometry.coordinates); return {n: f.properties.n, x: q.x, y: q.y}; }); }"""
        page.wait_for_function(f"({dots})().length > 1", timeout=20_000)            # the map has moved to the road
        first = page.input_value("#fix-via")
        target = next(d for d in page.evaluate(dots) if d["n"] != first and d["x"] > 450 and 60 < d["y"] < 780)
        page.wait_for_timeout(800)                                                  # the map stops
        target = next(d for d in page.evaluate(dots) if d["n"] == target["n"])
        page.mouse.click(target["x"], target["y"])
        page.wait_for_function(f"document.getElementById('fix-via').value === '{target['n']}'", timeout=15_000)
        page.wait_for_function("document.getElementById('fix-out').innerText.length > 0", timeout=15_000)
        assert page.input_value("#fix-from") == str(frm), "the click chose a junction; it did not pick the road again"
        assert page.evaluate("fixesPanel.drawn().turnJs") == len(listed)
        assert "white dot" in page.inner_text("#fix-via-row")
        assert not errors, errors
        browser.close()


def test_a_picked_object_is_never_called_missing_by_a_missing_row_or_a_failed_query(monaco, page_url):
    """A building that has walls but no row in features.buildings, and a node that is in a network but in no way list, are found; a query that FAILS says that the data could
    not be read, with the reason, and not "not in this area"."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        bid = int(page.evaluate(_QUERY, "SELECT building_id AS id FROM main.building_faces LIMIT 1")[0]["id"])
        def answer(patterns, status=200, body="[]"):
            page.unroute("**/api/query")
            page.route("**/api/query", lambda route: route.fulfill(status=status, content_type="application/json" if status == 200 else "text/plain", body=body)
                       if any(x in (route.request.post_data or "") for x in patterns) else route.continue_())
        text = lambda: page.inner_text("#fix-obj")
        # a building with walls and no row in features.buildings
        answer(["FROM features.buildings WHERE osm_id"])
        page.evaluate(f"fixesPanel.select('building', 'way', {bid})")
        page.wait_for_function("document.querySelector('#fix-obj .objhead')", timeout=15_000)
        page.wait_for_function("document.getElementById('fix-obj').innerText.includes('Only the walls')", timeout=15_000)
        assert "not in this area" not in text(), text()
        # a node that is in a network but in no way list, with no traffic row
        answer(["FROM features.traffic WHERE osm_id", "AND node_id IN (SELECT node_id FROM"])
        page.evaluate("fixesPanel.select('node', 'node', 21912099)")
        page.wait_for_function("document.getElementById('fix-obj').innerText.includes('A node of the')", timeout=15_000)
        assert "not in this area" not in text(), text()
        # a failed query: the data could not be read, and the reason is there
        answer(["FROM features.buildings WHERE osm_id"], status=400, body="Catalog Error: Table with name buildings does not exist")
        page.evaluate("fixesPanel.select('building', 'way', 999999999)")
        page.wait_for_function("document.getElementById('fix-obj').innerText.includes('could not be read')", timeout=15_000)
        assert "Catalog Error" in text() and "not in this area" not in text(), text()
        # and an ID that is really not there still says so
        page.unroute("**/api/query")
        page.evaluate("fixesPanel.select('building', 'way', 999999998)")
        page.wait_for_function("document.getElementById('fix-obj').innerText.includes('not in this area')", timeout=15_000)
        browser.close()


def test_wall_hover(monaco, page_url):
    """The Building tab: the pointer on a wall row lights the wall on the map and shows its number, and the row has the class "hover".
    The roadstyle hover of the tab (paintMapHover) is another function than the wall hover (paintHover): a name used twice made the wall hover do nothing."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        page.evaluate(f"window.fixesPanel.select('building', 'way', {WALL_BUILDING[0]})")
        page.wait_for_function("document.querySelectorAll('#fix-form .wallrow').length >= 3", timeout=30_000)
        page.wait_for_function("window.map.querySourceFeatures('fix-faces').length >= 3", timeout=30_000)
        page.evaluate("""() => { const c = window.map.querySourceFeatures('fix-faces').find((f) => f.geometry.type === 'LineString').geometry.coordinates[0];
            window.map.jumpTo({center: c, zoom: 19}); }""")
        page.wait_for_function("window.map.loaded() && !window.map.isMoving()", timeout=30_000)
        labels = "window.map.queryRenderedFeatures({layers: ['fix-faces-label']}).map((f) => f.properties.n)"
        page.wait_for_timeout(800)
        assert page.evaluate(labels) == [], "no wall number by default"
        row = page.locator("#fix-form .wallrow").nth(1)
        row.hover()
        page.wait_for_function(f"{labels}.length === 1", timeout=10_000)
        assert "hover" in row.get_attribute("class"), row.get_attribute("class")
        page.mouse.move(5, 5)
        page.wait_for_function(f"{labels}.length === 0", timeout=10_000)
        assert "hover" not in row.get_attribute("class")
        assert not errors, errors
        browser.close()
