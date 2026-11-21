"""Make the pictures of the editor for the docs (docs/images/edit-*.png). Each picture shows a picked object, never an empty panel.

Start the page on Monaco (a build with no fixes) in a new folder, so that the line over the tabs shows a short relative path:

    mkdir data && duckosm edit monaco.duckdb --load data/osm_fixes.yaml --save data/osm_fixes.yaml

then: python scripts/edit_page_pictures.py URL. The pictures:

- edit-page.png: the Road tab with a road picked;
- edit-road.png: the same road with "Lanes each way" changed to 3;
- edit-changes.png: the Changes tab with that change and Build again;
- edit-box.png: the road picked again: the box "This road is in your changes";
- edit-turn.png: the Turn tab with a road at a junction (the steps of tests/test_editor_turn_changes.py).

The size is 1400 x 900 at scale 1, so each file is less than about 600 kB."""
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

URL = sys.argv[1]
OUT = Path(__file__).resolve().parents[1] / "docs" / "images"
ROAD = 503462470                    # Boulevard Charles III (tests/editor_fixtures.py)
QUERY = "async (sql) => { const r = await fetch('api/query', {method: 'POST', body: sql}); return await r.json(); }"
SETTLED = "window.map.loaded() && !window.map.isMoving() && window.map.areTilesLoaded()"


def view(page, osm_id, zoom):
    """Tilt the camera over the road, in the part of the map at the right of the panel."""
    page.evaluate("""([id, z]) => { const rows = window.rsQuery((p) => Number(p.osm_id) === id); const b = window.RS_SIDE.bboxes[rows[0]];
        window.map.jumpTo({center: [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2], zoom: z, pitch: 60, bearing: -20,
                           padding: {left: 420, top: 0, right: 60, bottom: 0}}); }""", [osm_id, zoom])
    page.wait_for_function(SETTLED, timeout=30_000)
    page.wait_for_timeout(1500)


def pick(page, osm_id):
    """A click on the road: roadstyle's dashboard picks it, and the panel shows it."""
    page.evaluate("(id) => window.rsSelect(window.rsQuery((p) => Number(p.osm_id) === id)[0])", osm_id)
    page.wait_for_function(f"document.getElementById('fix-id').value === '{osm_id}' && document.querySelector('#fix-obj .objhead')", timeout=30_000)


def shot(page, name):
    page.screenshot(path=str(OUT / name))
    print(f"{name}: {(OUT / name).stat().st_size // 1024} kB")


with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_context(viewport={"width": 1400, "height": 900}).new_page()
    page.goto(URL)
    page.wait_for_function("window.fixesPanel && window.map && window.map._loaded", timeout=60_000)

    view(page, ROAD, 17.4)
    pick(page, ROAD)
    page.wait_for_timeout(800)
    shot(page, "edit-page.png")

    page.fill("#fixf-lanes", "3")
    page.wait_for_timeout(300)
    shot(page, "edit-road.png")

    page.click("#fix-save")
    page.click("#fix-unselect")
    page.click("#fix-tab-changes")
    page.wait_for_timeout(1500)
    shot(page, "edit-changes.png")

    page.click("#fix-tab-road")
    pick(page, ROAD)
    page.locator("#fix-inchg").wait_for(state="visible")
    page.wait_for_timeout(800)
    shot(page, "edit-box.png")

    # The Turn tab: a road that arrives at a junction where two other roads meet
    frm = int(page.evaluate(QUERY, "SELECT a.osm_id FROM driving.edges a JOIN driving.edges b ON b.source = a.target AND b.osm_id != a.osm_id "
                                   "JOIN driving.edges c ON c.target = a.target AND c.osm_id NOT IN (a.osm_id, b.osm_id) "
                                   "JOIN driving.edges d ON d.source = a.target AND d.osm_id NOT IN (a.osm_id, b.osm_id) LIMIT 1")[0]["osm_id"])
    page.evaluate("fixesPanel.tab('turns')")
    page.fill("#fix-from", str(frm))
    page.click("#fix-turn-find")
    page.wait_for_selector("#fix-out .turn", timeout=30_000)
    view(page, frm, 16.6)
    shot(page, "edit-turn.png")

    browser.close()
