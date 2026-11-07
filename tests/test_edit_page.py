"""``duckosm edit`` in a browser (docs/design/edit_page.md): the page of a Monaco build, a change of a road saved in osm_fixes.yaml, the box
"This road is in your changes" when the road is picked again, and Build again with the file. Headless Chromium (Playwright); the test
skips itself without Playwright and its browser. The map's base layers come from the internet."""
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
    return out / "monaco.duckdb"


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
