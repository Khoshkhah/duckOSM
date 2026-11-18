"""Make the picture of the editor for the docs: docs/images/edit-turn.png, the Turn tab with a road at a junction and its turns. Start `duckosm edit monaco.duckdb` first (Monaco, with no fixes), then: python scripts/edit_page_pictures.py URL.
The steps are the same as in tests/test_editor_turn_changes.py."""
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
OUT = Path(__file__).resolve().parents[1] / "docs" / "images"
QUERY = "async (sql) => { const r = await fetch('api/query', {method: 'POST', body: sql}); return await r.json(); }"
SETTLED = "window.map.loaded() && !window.map.isMoving() && window.map.areTilesLoaded()"


def view(page, osm_id, zoom):
    """Tilt the camera over the end of the road (the junction), in the part of the map at the right of the panel."""
    page.evaluate("""([id, z]) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); const b = window.RS_SIDE.bboxes[rows[0]];
        window.map.jumpTo({center: [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2], zoom: z, pitch: 60, bearing: -20,
                           padding: {left: 420, top: 0, right: 60, bottom: 0}}); }""", [osm_id, zoom])
    page.wait_for_function(SETTLED, timeout=30_000)
    page.wait_for_timeout(1500)


with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_context(viewport={"width": 1280, "height": 760}, device_scale_factor=2).new_page()
    page.goto(URL)
    page.wait_for_function("window.fixesPanel && window.map && window.map._loaded", timeout=60_000)

    # The Turn tab: a road that arrives at a junction where two other roads meet
    frm = int(page.evaluate(QUERY, "SELECT a.osm_id FROM driving.edges a JOIN driving.edges b ON b.source = a.target AND b.osm_id != a.osm_id "
                                   "JOIN driving.edges c ON c.target = a.target AND c.osm_id NOT IN (a.osm_id, b.osm_id) "
                                   "JOIN driving.edges d ON d.source = a.target AND d.osm_id NOT IN (a.osm_id, b.osm_id) LIMIT 1")[0]["osm_id"])
    page.evaluate("fixesPanel.tab('turns')")
    page.fill("#fix-from", str(frm))
    page.click("#fix-turn-find")
    page.wait_for_selector("#fix-out .turn", timeout=30_000)
    view(page, frm, 16.6)
    page.screenshot(path=str(OUT / "edit-turn.png"))

    browser.close()
