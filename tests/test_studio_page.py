"""``duckosm studio`` in a browser (docs/guides/studio.md): the six steps on Monaco, from the area to the page of the result. Headless Chromium (Playwright);
the test skips itself without Playwright and its browser. The Geofabrik list and the Monaco extract come from a small local server (``--geofabrik-index``),
Nominatim's answer is given by the test, and the tiles of the base map are not loaded. The page of the result loads js-yaml from the internet, as the page of
``duckosm edit`` does. The unit tests of the logic are ``web/build_steps/steps_units.mjs``; the rules of the server are the tests of rust/duckosm-edit/src/studio.rs."""
import http.server
import json
import re
import socket
import subprocess
import sys
import threading
from pathlib import Path

import duckdb
import pytest

pw = pytest.importorskip("playwright.sync_api")
ROOT = Path(__file__).resolve().parents[1]
PBF = ROOT / "data" / "sample" / "monaco.osm.pbf"
MONACO = json.loads((ROOT / "data" / "sample" / "monaco.geojson").read_text())


def _box(w, s, e, n):
    return {"type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]}


def _free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(scope="module")
def mirror():
    """A server for the list of the Geofabrik extracts and the Monaco file: the list has Monaco and Europe, with the address of the file on this server."""
    state = {}

    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/index-v1.json":
                url = state["url"]
                body = json.dumps({"type": "FeatureCollection", "features": [
                    {"type": "Feature", "properties": {"id": "europe", "name": "Europe", "urls": {"pbf": f"{url}/europe-latest.osm.pbf"}}, "geometry": _box(-30, 30, 50, 75)},
                    {"type": "Feature", "properties": {"id": "monaco", "name": "Monaco", "parent": "europe", "urls": {"pbf": f"{url}/monaco-latest.osm.pbf"}},
                     "geometry": _box(7.38, 43.70, 7.48, 43.78)}]}).encode()
                kind = "application/json"
            elif self.path == "/monaco-latest.osm.pbf":
                body, kind = PBF.read_bytes(), "application/octet-stream"
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    state["url"] = f"http://127.0.0.1:{srv.server_address[1]}"
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield state["url"]
    srv.shutdown()


@pytest.fixture(scope="module")
def studio(tmp_path_factory, mirror):
    """`duckosm studio` on a new folder: (the folder of the areas, the address of the page)."""
    out = tmp_path_factory.mktemp("studio")
    port = _free_port()
    log = out.parent / "studio.log"
    with open(log, "w") as f:
        proc = subprocess.Popen([sys.executable, "-c", "from duckosm.cli import main; main()", "studio", str(out), "--port", str(port), "--no-open",
                                 "--geofabrik-index", f"{mirror}/index-v1.json"], stdout=f, stderr=subprocess.STDOUT, text=True)
        url = None
        for _ in range(300):
            m = re.search(r"http://\S+", log.read_text())
            if m or proc.poll() is not None:
                url = m and m.group(0)
                break
            subprocess.run(["sleep", "0.1"])
    if not url:
        proc.kill()
        pytest.fail(f"duckosm studio did not start: {log.read_text()}")
    yield out, url
    proc.terminate()
    proc.wait(10)


def _open(browser, url, **ctx):
    """The page with Nominatim's answer given (Monaco, and a point with no outline) and no tiles; the errors of the page are collected."""
    context = browser.new_context(**ctx)
    context.route("https://tile.openstreetmap.org/**", lambda r: r.abort())
    # the search of Studio: Photon lists the places, Nominatim gives the boundary of each (a node has none)
    cors = {"Access-Control-Allow-Origin": "*"}
    photon = {"type": "FeatureCollection", "features": [
        {"type": "Feature", "geometry": {"type": "Point", "coordinates": [7.42, 43.73]}, "properties": {"osm_type": "R", "osm_id": 1124039, "name": "Monaco", "type": "country", "country": "Monaco", "extent": [7.40, 43.76, 7.45, 43.72]}},
        {"type": "Feature", "geometry": {"type": "Point", "coordinates": [7.42, 43.73]}, "properties": {"osm_type": "N", "osm_id": 5, "name": "Monaco cafe", "type": "other", "osm_value": "cafe"}}]}
    lookup = {"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {"osm_type": "relation", "osm_id": 1124039, "display_name": "Monaco, Europe", "type": "administrative"},
                                                          "geometry": MONACO["features"][0]["geometry"]}]}
    context.route("https://photon.komoot.io/**", lambda r: r.fulfill(status=200, content_type="application/json", headers=cors, body=json.dumps(photon)))
    context.route("https://nominatim.openstreetmap.org/**", lambda r: r.fulfill(status=200, content_type="application/json", headers=cors, body=json.dumps(lookup)))
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(url)
    page.wait_for_function("window.studioReady", timeout=30_000)
    return page, errors


def _unfold(page, *steps):
    """Open the cards of these steps (a card that is not the open step is folded, as in Studio; Playwright cannot use a hidden field)."""
    page.evaluate("(ns) => ns.forEach((n) => rs.sections.fold('step' + n, false))", list(steps))


def _options(page):
    """The settings: the card Build is open, and the box More options too."""
    _unfold(page, 5)
    page.evaluate("document.querySelectorAll('.bs-settings details.drow').forEach((d) => { d.open = true; })")


def _fit(page):
    assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth"), "the page is wider than the screen"


def _build_and_open(page, area, timeout=600_000):
    """Push Build, wait for the result, and open the map of the new area: the page of `duckosm edit` at /edit/."""
    _unfold(page, 5)
    page.click("[data-bs=run]")
    _unfold(page, 6)
    page.wait_for_selector(".bs-open", state="visible", timeout=timeout)
    assert page.is_hidden("[data-bs=error]")
    assert area in page.inner_text("[data-rs=line]")
    with page.expect_navigation(timeout=60_000):
        page.click(".bs-open")
    page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
    assert page.url.endswith("/edit/")
    assert page.evaluate("fetch('api/info').then(r => r.json()).then(i => i.name)") == area


def test_a_place_a_geofabrik_extract_and_no_dem(studio):
    """Search → Monaco, the Geofabrik extract (downloaded from the local server), settings with two networks, no DEM, build, and the page of the result."""
    out, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.fill("[data-as=search] input", "Monaco")                 # no Enter: the search starts while the person types
        page.wait_for_selector("[name=bs-place]")
        page.wait_for_function("document.querySelectorAll('[name=bs-place]').length === 2")
        assert page.locator("[name=bs-place]:not([disabled])").count() == 1 and page.locator("[name=bs-place][disabled]").count() == 1, "a place with no boundary is listed with no choice"
        assert page.is_disabled("[data-bs=run]") and "Choose the area" in page.inner_text("[data-bs=why]")
        page.click("[name=bs-place] + span")
        assert page.input_value("[data-as=name]") == "Monaco"
        _unfold(page, 2)
        page.fill("[data-as=name]", "monaco-a")
        assert "km²" in page.inner_text("[data-as=kpis]") and "vertices" in page.inner_text("[data-as=kpis]").lower()
        # the extracts that cover the area: Monaco first, Europe second; the first is chosen
        page.wait_for_selector("[name=bs-extract]")
        assert page.eval_on_selector_all("[name=bs-extract]", "els => els.map(e => e.value)") == ["monaco", "europe"]
        assert page.eval_on_selector("[name=bs-extract]:checked", "e => e.value") == "monaco"
        assert page.is_enabled("[data-bs=run]")
        # the settings: two networks; the text of the config follows
        _options(page)
        page.uncheck("[name=mode][value=cycling]")
        page.uncheck("[name=opt][value=levels]")
        assert "modes: [driving, walking]" in page.text_content("[data-ss=yaml]")
        # no network at all: the build is off, with the reason
        for m in ("driving", "walking"):
            page.uncheck(f"[name=mode][value={m}]")
        assert page.is_disabled("[data-bs=run]") and "at least one network" in page.inner_text("[data-bs=why]")
        for m in ("driving", "walking"):
            page.check(f"[name=mode][value={m}]")
        # the ground: no source is chosen, so there is no DEM
        assert page.query_selector("[name=gs-src]:checked") is None
        assert page.evaluate("window.studio.ground.dem()") is None
        _fit(page)
        _build_and_open(page, "monaco-a")
        browser.close()
        assert not errors, errors
    folder = out / "monaco-a"
    assert (folder / "monaco-latest.osm.pbf").read_bytes() == PBF.read_bytes(), "the extract was downloaded"
    assert (folder / "boundary.geojson").is_file() and not (folder / "dem.tif").exists()
    config = (folder / "config.yaml").read_text()
    assert "modes: [driving, walking]" in config and "elevation:" not in config and "monaco-latest.osm.pbf" in config, config
    con = duckdb.connect(str(folder / "monaco-a.duckdb"), read_only=True)
    schemas = {r[0] for r in con.execute("SELECT DISTINCT table_schema FROM information_schema.tables").fetchall()}
    assert {"driving", "walking"} <= schemas and "cycling" not in schemas, schemas
    assert con.execute("SELECT count(*) FROM driving.edges").fetchone()[0] > 1000
    assert con.execute("SELECT count(*) FROM information_schema.tables WHERE table_name IN ('elevation_grid', 'elevation_metadata')").fetchone()[0] == 0
    record = con.execute("SELECT build_config, build_dir FROM main.visualization_metadata").fetchone()
    assert record[0] == config and record[1] == str(folder), "Build again has what it needs"
    con.close()


def test_a_file_a_local_pbf_and_a_dem(studio, tmp_path):
    """A boundary file, a PBF on this computer, and a DEM file from a source of the catalog: the build has main.elevation_metadata and main.elevation_grid."""
    rasterio = pytest.importorskip("rasterio")
    import numpy as np
    from rasterio.transform import from_origin
    out, url = studio
    dem = tmp_path / "fabdem.tif"
    with rasterio.open(dem, "w", driver="GTiff", width=30, height=30, count=1, dtype="float32", crs="EPSG:4326", transform=from_origin(7.38, 43.78, 0.003, 0.002),
                       nodata=-9999.0) as ds:
        ds.write(np.full((1, 30, 30), 42, "float32"))
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.click("[data-tab=file]")
        page.set_input_files("[data-as=drop] input[type=file]", str(ROOT / "tests" / "data" / "monaco.geojson"))
        page.wait_for_function("window.studio.area.area()")
        assert page.input_value("[data-as=name]") == "Monaco"
        _unfold(page, 2)
        page.fill("[data-as=name]", "monaco-dem")
        # the data: a file on this computer; a path that is not a file says so
        page.locator(".bs-data [data-tab=file]").click()
        page.fill("[data-ds=path]", str(tmp_path / "nothing.osm.pbf"))
        page.wait_for_function("document.querySelector('[data-ds=msg]').textContent.includes('not a file')")
        assert page.is_disabled("[data-bs=run]")
        page.fill("[data-ds=path]", str(PBF))
        page.wait_for_function("document.querySelector('[data-ds=msg]').textContent.startsWith('Found')")
        assert page.is_enabled("[data-bs=run]")
        # fewer steps in this build: the DEM is what this test is about
        _options(page)
        for k in ("levels", "bus_routes", "build_features"):
            page.uncheck(f"[name=opt][value={k}]")
        for m in ("walking", "cycling"):
            page.uncheck(f"[name=mode][value={m}]")
        # the ground: FABDEM (a DTM, EGM2008) and its file
        _unfold(page, 4)
        page.check("[name=gs-src][value=fabdem]")
        page.set_input_files("[data-gs=file]", str(dem))
        page.wait_for_selector("[data-gs=remove]")
        assert dem.name in page.inner_text(".gs") and "EGM2008" in page.inner_text(".gs")
        page.wait_for_function("/^[0-9a-f]{64}$/.test(document.querySelector('[data-gs=sha]').textContent)")
        _build_and_open(page, "monaco-dem")
        browser.close()
        assert not errors, errors
    folder = out / "monaco-dem"
    assert (folder / "dem.tif").read_bytes() == dem.read_bytes()
    config = (folder / "config.yaml").read_text()
    assert 'dem: dem.tif' in config and 'product: "DTM"' in config and 'vertical_datum: "EGM2008"' in config and "reference: true" in config, config
    con = duckdb.connect(str(folder / "monaco-dem.duckdb"), read_only=True)
    meta = con.execute("SELECT product, vertical_datum, license, sha256 FROM main.elevation_metadata").fetchall()
    assert len(meta) == 1 and meta[0][:3] == ("DTM", "EGM2008", "CC-BY-NC-SA-4.0") and re.fullmatch(r"[0-9a-f]{64}", meta[0][3]), meta
    assert con.execute("SELECT count(*) FROM main.elevation_grid WHERE z IS NOT NULL").fetchone()[0] > 100
    assert con.execute("SELECT count(*) FROM driving.nodes WHERE ele IS NOT NULL").fetchone()[0] > 100
    con.close()


def test_a_box_on_a_phone(studio):
    """390 × 844: the panel is roadstyle's sheet at the bottom; a box drawn on the map (Draw) makes the area and the extracts; nothing is wider than the screen."""
    out, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 390, "height": 844}, device_scale_factor=2)
        _fit(page)
        page.evaluate("map.jumpTo({ center: [10, 50], zoom: 3 })")      # the page opens on the world: a person zooms to Europe before the box
        page.click("[data-tab=draw]")
        assert page.locator(".rs-toolrow button").count() == 8, "the tools of Studio: rectangle, polygon, circle, freehand, select, merge, delete, clear"
        page.wait_for_function("document.querySelector('[data-mode=rectangle]').getAttribute('aria-pressed') === 'true'")      # the rectangle is the tool at the start
        assert page.evaluate("window.studio.area.drawMode()") == "rectangle"        # terra-draw has loaded and is in this mode
        page.mouse.move(120, 120)
        page.mouse.down()
        page.mouse.move(200, 160, steps=5)
        page.mouse.move(270, 230, steps=5)
        page.mouse.up()
        page.wait_for_function("window.studio.area.area()", timeout=30_000)
        a = page.evaluate("window.studio.area.area()")
        assert a["from"] == "drawn" and a["name"] == "drawn_area" and a["km2"] > 1000, a
        page.wait_for_selector("[name=bs-extract]")
        assert page.eval_on_selector("[name=bs-extract]:checked", "e => e.value") == "europe"
        assert page.is_enabled("[data-bs=run]")
        _fit(page)
        # the Build card is in the sheet: it can be scrolled to, and its button is a finger's size
        _unfold(page, 5)
        page.locator("[data-bs=run]").scroll_into_view_if_needed()
        assert page.locator("[data-bs=run]").bounding_box()["height"] >= 44
        page.screenshot(path=str(out / "phone.png"))
        assert not errors, errors
        browser.close()


def test_saved_areas_are_listed_loaded_and_opened(studio):
    """The areas are on the disk: a new visit lists them (the card Your areas), Load puts the area and the settings of one back into the steps, and Open shows its map
    and the editor. It uses the area of the first test (the tests run in the order of the file)."""
    out, url = studio
    if not (out / "monaco-a" / "monaco-a.duckdb").exists():
        pytest.skip("needs the area that the first test builds")
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.wait_for_selector(".saved-area")
        assert "monaco-a" in page.eval_on_selector_all(".saved-area", "els => els.map(e => e.dataset.name)")
        row = page.locator(".saved-area[data-name=monaco-a]")
        text = row.inner_text()
        assert "driving" in text and "walking" in text and "cycling" not in text and "MB" in text, text
        row.locator("[data-do=load]").click()
        page.wait_for_function("window.studio.area.area()?.name === 'monaco-a'")
        assert page.evaluate("window.studio.settings.values().modes") == ["driving", "walking"]
        assert page.evaluate("window.studio.settings.values().levels") is False and page.evaluate("window.studio.settings.values().build_features") is True
        assert "is loaded" in page.inner_text("#sb")
        with page.expect_navigation(timeout=60_000):
            row.locator("[data-do=open]").click()
        page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
        assert page.url.endswith("/edit/") and page.evaluate("fetch('api/info').then(r => r.json()).then(i => i.name)") == "monaco-a"
        browser.close()
        assert not errors, errors


def test_a_page_with_no_roads_has_no_patch_of_the_page(studio):
    """The page has no roads before a build, and uses roadstyle's dashboard unchanged: no banner "0 road features", a first view of the world (not 0, 0),
    and the option rows (the networks) in the type of an option, not in the capitals of a label (roadstyle PR 153)."""
    _, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url)
        _options(page)
        page.wait_for_timeout(3500)                          # roadstyle's check for a map with no road features waits 2.5 s
        assert page.locator("#rs-diag").count() == 0
        assert page.evaluate("map.getZoom()") < 4 and page.evaluate("map.getCenter().lat") != 0
        assert page.evaluate("getComputedStyle(document.querySelector('#sb .rs-opt')).textTransform") == "none"
        browser.close()
        assert not errors, errors
