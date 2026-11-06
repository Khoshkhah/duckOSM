"""``duckosm edit`` in a browser (docs/design/edit_page.md): the page of a Monaco build, the editor in roadstyle's editor frame (the tabs in the
head row, the bar of each tab at the bottom edge, the back button), the roads in roadstyle's highlight sets, a change of a road saved in
osm_fixes.yaml, the box "This road is in your changes" when the road is picked again, and Build again with the file. Headless Chromium
(Playwright); the test skips itself without Playwright and its browser. The map's base layers come from the internet."""
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
BLUE, AMBER = "#2563eb", "#f59e0b"    # the sets fix-sel and fix-edits (ROAD_SETS of fixes_panel.js)
# the colour of the highlight set on each map row of the road (the feature state rs_hl that roadstyle's road layer reads)
_LIT = """(id) => window.rsQuery((p) => Number(p.osm_id) === id).map((row) => { const l = window.map.getLayer("roads-simple") ?? window.map.getLayer("roads-fill");
    return window.map.getFeatureState({source: l.source, sourceLayer: l.sourceLayer, id: row}).rs_hl ?? null; })"""


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


def _bars(page):
    """The bars of the tabs that show in the frame's bar, and whether the frame's bar shows."""
    return page.evaluate("""() => [[...document.querySelectorAll('#fix-panel > .rs-frame-bar > .fix-bar')].filter((e) => e.offsetParent).map((e) => e.id),
        !!document.querySelector('#fix-panel > .rs-frame-bar').offsetParent]""")


def test_edit_save_box_and_build(monaco, page_url):
    fixes = monaco.with_name("osm_fixes.yaml")
    assert not fixes.exists()
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url)
        # the editor is in roadstyle's editor frame in the dashboard's panel: the title line (back button, title, file name), the six tabs in the
        # head row, the body that scrolls and the bar; the host's own title and body are hidden
        assert page.locator("#sb .sb-top").is_visible() and page.locator("#fix-panel.rs-frame").is_visible()
        assert page.inner_text(".rs-frame-title .t") == "Edit OSM data" and page.inner_text(".rs-frame-title .fix-file") == "osm_fixes.yaml"
        assert page.locator("#fix-tabs [role=tab]").count() == 6 and page.locator(".fix-tabs").count() == 0
        assert not page.locator("#sb .sb-body").is_visible()
        assert _bars(page) == [[], False], "no pick: the bar does not show"
        _pick_road(page, ROAD)
        assert _bars(page) == [["fix-actions"], True]
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
        # an edited road is amber when it is not picked (the set fix-edits)
        page.click("#fix-unselect")
        page.wait_for_function(f"({_LIT})({ROAD}).every((c) => c === '{AMBER}')", timeout=10_000)
        # the bar changes with the tab: Turn with no road has no bar, Changes has Build again
        page.click("#fix-tab-turns")
        page.locator("#fix-status").wait_for(state="hidden")          # the status line ("Saved in ...") is in the bar too
        assert _bars(page) == [[], False]
        page.click("#fix-tab-changes")
        assert _bars(page) == [["fix-build-bar"], True]
        # the back button closes the editor and shows the dashboard's own panel; its card "Edit OSM data" opens the editor again
        # ponytail: a DOM click. On a desktop, roadstyle's Layers box (overlays.js: a page with no toolbar puts it in the top-left stack .rs-tl) is over
        # the back button, so a click at the button's place hits the box. Use page.click when roadstyle docks that box.
        page.locator("#fix-back").dispatch_event("click")
        assert not page.locator("#fix-panel").is_visible() and page.locator("#sb .sb-body").is_visible()
        assert not page.evaluate("document.body.classList.contains('editing-fixes')")
        assert page.evaluate(f"({_LIT})({ROAD})")[0] == AMBER, "the changes stay on the map"
        page.click("#sb [data-rs-section=fixes] .rs-sec-title")
        page.locator("#fix-panel").wait_for(state="visible")
        page.click("#fix-tab-road")
        # the road again: the box first, and the form waits
        _pick_road(page, ROAD)
        box = page.locator("#fix-inchg")
        box.wait_for(state="visible")
        assert "This road is in your changes" in box.inner_text() and "lanes 3" in box.inner_text()
        assert not page.locator("#fix-form").is_visible()
        assert _bars(page) == [[], False], "the bar waits for the box too"
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


def test_phone(monaco, page_url):
    """The page on a phone: the dashboard's sheet has the editor's frame at its full height, the tab row scrolls sideways, the bar is at the
    bottom edge, and nothing is wider than the screen."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, page_url, viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
        assert page.evaluate("document.documentElement.classList.contains('rs-phone')")
        page.locator("#fix-tabs").wait_for(state="visible")
        assert page.evaluate("window.rs.sheet()") == 2, "the frame opens the sheet to its full height"
        assert page.evaluate("document.documentElement.scrollWidth") <= 390
        row = page.evaluate("(() => { const r = document.getElementById('fix-tabs'); return [r.getBoundingClientRect().right, r.scrollWidth > r.clientWidth]; })()")
        assert row[0] <= 390 and row[1], row
        _pick_road(page, ROAD)
        if page.locator("#fix-inchg").is_visible():           # the road is in the changes of the test before: the bar waits for the box
            page.click("#fix-inchg [data-ask=edit]")
        bar = page.evaluate("document.querySelector('#fix-panel > .rs-frame-bar').getBoundingClientRect().toJSON()")
        assert bar["right"] <= 390 and 844 - 40 <= bar["bottom"] <= 844, bar
        assert not errors, errors
        browser.close()


def test_host_without_road_rows(monaco, page_url):
    """A host that does not give roadRows yet (the old contract): the picked road is drawn by the editor's own line layer, not by a highlight set."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context()
        # the page has its script inline (rust/duckosm-edit/src/page.rs): the page without roadRows in the editor's ctx
        ctx.route(page_url, lambda route: route.fulfill(response=(r := route.fetch()), body=r.text().replace("roadIdOfRow, roadRows,", "roadIdOfRow,")))
        page = ctx.new_page()
        page.goto(page_url)
        page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
        _pick_road(page, ROAD)
        page.wait_for_function("window.fixesPanel.drawn().sel > 0", timeout=10_000)
        assert page.evaluate("window.fixesPanel.drawn()")["lit"]["fix-sel"] == 0
        assert page.evaluate(f"({_LIT})({ROAD})") == [None] * 2
        browser.close()
