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
    _unfold(page, 4)
    page.evaluate("document.querySelectorAll('.bs-settings details.drow').forEach((d) => { d.open = true; })")


def _fit(page):
    assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth"), "the page is wider than the screen"


OPEN = "(n) => window.studio.area.area()?.name === n && document.querySelector('#sb').innerText.includes(n + ' is open')"


def _build_and_open(page, area, timeout=600_000):
    """Push Build (step 4) and wait for the result: the page does not load again, and the map of the new area shows on its own map (the tiles of the server,
    the blank base map). Then open the editor from card 7 (the page of `duckosm edit` at /edit/); its back button comes back to the steps, with the same area."""
    _unfold(page, 4)
    page.evaluate("window.__same = 1")
    page.click("#build-box [data-bs=run]")
    page.wait_for_function(OPEN, arg=area, timeout=timeout)
    assert page.evaluate("window.__same") == 1, "the page did not load again"
    assert page.is_hidden("#build-box [data-bs=error]")
    _unfold(page, 5)
    assert area in page.inner_text("[data-rs=line]")
    # the result shows where the files are: the folder of the area and each file with its size
    assert area in page.inner_text(".bs-result [data-rs=dir]") and page.inner_text(".bs-result [data-rs=dir]").startswith("/")
    files = page.eval_on_selector_all(".bs-files tbody tr", "els => els.map((r) => r.cells[0].textContent)")
    assert f"{area}.duckdb" in files and "config.yaml" in files and "boundary.geojson" in files, files
    # the build details: the time of each step, and the total
    steps = page.evaluate("[...document.querySelectorAll('#build-box .bs-steplog tbody tr')].map((r) => [r.cells[0].textContent, r.cells[1].textContent])")
    assert len(steps) > 3 and all(re.fullmatch(r"\d+\.\d", sec) for _, sec in steps), steps
    # the map of the area on this page: its tiles from the server, the blank base map, only the line of the boundary
    assert page.url.endswith(f"?area={area}")
    page.wait_for_function("window.map.getStyle().layers.some((l) => l.id.startsWith('rs-'))", timeout=60_000)
    assert page.evaluate("window.map.getLayoutProperty('bs-area-fill', 'visibility')") == "none"
    # card 7: the editor of the area, and back
    with page.expect_navigation(timeout=120_000):
        page.click("[data-rs-section=step7] .rs-sec-title")
    page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=120_000)
    assert page.url.endswith("/edit/") and page.evaluate("fetch('api/info').then(r => r.json()).then(i => i.name)") == area
    page.click("#rs-frame-back, .rs-frame-back")
    page.wait_for_function("window.studioReady", timeout=60_000)
    page.wait_for_function(OPEN, arg=area, timeout=120_000)


def _wait_job(url, done, timeout=600):
    """Wait until the server's build job is as `done` says (a fetch in wait_for_function gives a Promise, which counts as true at once)."""
    import time
    import urllib.request
    end = time.time() + timeout
    while time.time() < end:
        j = json.loads(urllib.request.urlopen(url + "api/job").read())
        if done(j):
            assert j["state"] == "done", j.get("error")
            return j
        time.sleep(1)
    raise AssertionError("the build did not end")


def _download(page, extract):
    """Download an extract in step 3 (into the folder of the areas): the row's Download, the progress, then In use."""
    _unfold(page, 3)
    page.click(f"[data-ds-get={extract}]")
    page.wait_for_function(f"[...document.querySelectorAll('.bs-data .rs-opt')].some((r) => r.querySelector('[value={extract}]') && /In use/.test(r.textContent))", timeout=120_000)


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
        # the place under the mouse shows its outline on the map (the purple preview of Studio), and it goes when the mouse leaves the list
        preview = "(() => { try { return window.studio.map.getSource('bs-preview').serialize().data.features.length; } catch { return -1; } })()"      # -1: the layers are not made yet
        page.hover(".bs-area label[data-i='0']")
        page.wait_for_function(f"{preview} === 1")
        page.wait_for_function("window.studio.map.getZoom() > 6", timeout=30_000)      # the place is small on the world map: the map moves to it (Studio's rule)
        page.mouse.move(900, 400)
        page.wait_for_function(f"{preview} === 0")
        # a place with no boundary (a point) shows too, and a click on it moves the map to it
        page.evaluate("window.studio.map.jumpTo({center: [0, 0], zoom: 2})")
        page.hover(".bs-area label.point")
        page.wait_for_function(f"{preview} === 1")
        page.wait_for_function("window.studio.map.getZoom() > 6", timeout=30_000)
        page.evaluate("window.studio.map.jumpTo({center: [0, 0], zoom: 2})")
        page.click(".bs-area label.point", force=True)
        page.wait_for_function("window.studio.map.getZoom() > 6", timeout=30_000)
        page.mouse.move(900, 400)
        page.wait_for_function(f"{preview} === 0")
        page.click("[name=bs-place] + span")
        page.wait_for_function("window.studio.area.area()")
        assert page.evaluate(preview) == 0, "after a pick only the area shows"
        assert page.input_value("[data-as=name]") == "Monaco"
        _unfold(page, 2)
        page.fill("[data-as=name]", "monaco-a")
        assert "km²" in page.inner_text("[data-as=kpis]") and "vertices" in page.inner_text("[data-as=kpis]").lower()
        # the extracts that cover the area: Monaco first, Europe second; the first is chosen
        page.wait_for_selector("[name=bs-extract]")
        assert page.eval_on_selector_all("[name=bs-extract]", "els => els.map(e => e.value)") == ["monaco", "europe"]
        assert page.query_selector("[name=bs-extract]:checked") is None, "nothing is chosen at the start, as in Studio"
        _unfold(page, 3)
        # no hover (it was in the way); a click shows the extract on the map (orange, dashed), a second click hides it and takes the choice away (Studio)
        ext = "(() => { try { return window.studio.map.getSource('bs-extract').serialize().data.features.length; } catch { return -1; } })()"
        page.hover("[name=bs-extract][value=monaco] + span")
        page.wait_for_timeout(500)
        assert page.evaluate(ext) in (-1, 0), "the mouse over an extract shows nothing"
        page.click("[name=bs-extract][value=monaco] + span")
        page.mouse.move(900, 400)
        assert page.evaluate(ext) == 1 and page.eval_on_selector("[name=bs-extract]:checked", "e => e.value") == "monaco", "a click keeps the outline"
        page.click("[name=bs-extract][value=monaco] + span")
        page.mouse.move(900, 400)
        page.wait_for_function(f"{ext} === 0")
        assert page.query_selector("[name=bs-extract]:checked") is None and page.is_disabled("[data-bs=run]"), "a second click takes the choice away"
        page.click("[name=bs-extract][value=monaco] + span")
        assert page.eval_on_selector("[name=bs-extract]:checked", "e => e.value") == "monaco"
        # a chosen extract that is not downloaded is not the data yet; Download puts it into the folder, and it is in use
        assert page.is_disabled("[data-bs=run]") and "Download" in page.inner_text("[data-bs=why]")
        _download(page, "monaco")
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
    assert (out / "osm" / "monaco-latest.osm.pbf").read_bytes() == PBF.read_bytes(), "step 3 downloaded the extract into the folder of the areas"
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
        _build_and_open(page, "monaco-dem")
        # the ground after the build (step 6): FABDEM (a DTM, EGM2008) and its file; Add the ground builds the area again with it
        _unfold(page, 6)
        page.check("[name=gs-src][value=fabdem]")
        page.set_input_files("[data-gs=file]", str(dem))
        page.wait_for_selector("[data-gs=remove]")
        assert dem.name in page.inner_text(".gs") and "EGM2008" in page.inner_text(".gs")
        page.wait_for_function("/^[0-9a-f]{64}$/.test(document.querySelector('[data-gs=sha]').textContent)")
        page.evaluate("window.__same = 1")
        page.click("[data-rs-section=step6] [data-bs=run]")
        _wait_job(url, lambda j: j["state"] == "error" or (j["state"] == "done" and any(f["name"] == "dem.tif" for f in j["files"])))
        assert page.evaluate("window.__same") == 1
        page.wait_for_function(OPEN, arg="monaco-dem", timeout=120_000)
        assert page.evaluate("window.__same") == 1
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


def test_card_8_edits_the_levels_of_an_area(studio):
    """Card 8, Edit levels: roadstyle's level-edit part over the area's map. It opens from the stored levels, a change is drawn again by the server
    (way D, POST relevel), and "← Steps" writes the change into the area's .duckdb file. At 390 x 844 the editor opens in the sheet."""
    out, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        asked = []
        page.on("request", lambda r: "/areas/" in r.url and "/tiles/" not in r.url and asked.append(r.method + " " + r.url.split("/areas/")[1]))
        page.click("[data-tab=file]")
        page.set_input_files("[data-as=drop] input[type=file]", str(ROOT / "tests" / "data" / "monaco.geojson"))
        page.wait_for_function("window.studio.area.area()")
        _unfold(page, 2)
        page.fill("[data-as=name]", "monaco-lv")
        page.locator(".bs-data [data-tab=file]").click()
        page.fill("[data-ds=path]", str(PBF))
        page.wait_for_function("document.querySelector('[data-ds=msg]').textContent.startsWith('Found')")
        _options(page)
        for k in ("bus_routes", "build_features"):                 # the levels stay: card 8 edits them
            page.uncheck(f"[name=opt][value={k}]")
        for m in ("walking", "cycling"):
            page.uncheck(f"[name=mode][value={m}]")
        _build_and_open(page, "monaco-lv")
        card = "[data-rs-section=step8]"
        page.click(card + " .rs-sec-title")
        page.wait_for_function("document.body.dataset.rsEditing === 'levels' || /failed/.test(document.querySelector('[data-rs-section=step8]').innerText)", timeout=300_000)
        assert "open in" in page.inner_text(card), page.inner_text(card)
        assert page.evaluate("window.studio.levelPart.editor.answer().resolve.how") == "given", "the editor starts from the stored levels"
        road = page.evaluate("(async () => { const e = window.studio.levelPart.editor, ids = e.roads.slice(0, 300), f = await e.facts(ids); return ids.find((i) => f[i].length_m > 30); })()")
        said = page.evaluate("r => window.lvApi('/api/apply', {ops: [{op: 'head', road: r, end: 'end', m: '7'}]})", road)
        assert said["ok"] and said["redrawn"], said
        assert "POST monaco-lv/relevel" in asked, ("the server draws the changed road again", asked)
        page.click("#lv-back")
        page.wait_for_function("/Applied|failed/.test(document.querySelector('[data-rs-section=step8]').innerText)", timeout=120_000)
        assert "Applied" in page.inner_text(card), page.inner_text(card)
        browser.close()
        assert not errors, errors
    con = duckdb.connect(str(out / "monaco-lv" / "monaco-lv.duckdb"), read_only=True)
    assert con.execute("SELECT road::VARCHAR, end_m FROM visualization.level_heads").fetchall() == [(str(road), "7")]
    assert con.execute("SELECT source, area FROM visualization.edge_levels_meta").fetchall() == [("area", "monaco-lv")]
    con.close()
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url + "?area=monaco-lv", viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
        page.wait_for_function(OPEN, arg="monaco-lv", timeout=120_000)
        page.click("[data-rs-section=step8] .rs-sec-title")
        page.wait_for_function("document.body.dataset.rsEditing === 'levels'", timeout=300_000)
        assert page.evaluate("window.studio.levelPart.editor.answer().resolve.how") == "given", "a new browser starts from the levels of the file (with the change)"
        assert page.is_visible("#lv")
        _fit(page)
        page.click("#lv-back")
        page.wait_for_function("!document.body.dataset.rsEditing", timeout=60_000)
        browser.close()
        assert not errors, errors


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
        _unfold(page, 3)
        page.click("[name=bs-extract][value=europe] + span")
        assert page.eval_on_selector("[name=bs-extract]:checked", "e => e.value") == "europe"
        assert page.is_disabled("[data-bs=run]") and "Download" in page.inner_text("[data-bs=why]"), "Europe is not downloaded: the build waits"
        page.locator(".bs-data [data-tab=file]").click()                 # a file on this computer: the build is ready
        page.fill("[data-ds=path]", str(PBF))
        page.wait_for_function("document.querySelector('[data-ds=msg]').textContent.startsWith('Found')")
        assert page.is_enabled("[data-bs=run]")
        _fit(page)
        # the Build card is in the sheet: it can be scrolled to, and its button is a finger's size
        _unfold(page, 4)
        page.locator("#build-box [data-bs=run]").scroll_into_view_if_needed()
        assert page.locator("#build-box [data-bs=run]").bounding_box()["height"] >= 44
        page.screenshot(path=str(out / "phone.png"))
        assert not errors, errors
        browser.close()


def test_h3_cells_are_the_area(studio):
    """The H3 tab of Studio: a hexagon that the person clicks is the area; a second click takes it away; a cell ID in the search box makes the area."""
    out, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.evaluate("window.studio.map.jumpTo({center: [7.42, 43.735], zoom: 12})")
        page.click("[data-tab=h3]")
        page.wait_for_function("document.querySelector('[data-as=h3-level]').options.length === 6", timeout=60_000)      # h3-js has loaded
        page.evaluate("() => { const s = document.querySelector('[data-as=h3-level]'); s.value = '8'; s.dispatchEvent(new Event('change', { bubbles: true })); }")      # roadstyle draws its own list over the select
        page.wait_for_function("document.querySelector('[data-as=h3-grid-note]').textContent.includes('hexagon')", timeout=30_000)
        page.mouse.click(900, 450)
        page.wait_for_function("window.studio.area.area()?.from === 'h3'", timeout=30_000)
        a = page.evaluate("window.studio.area.area()")
        assert re.fullmatch(r"[0-9a-f]{15}", a["name"]) and 0.5 < a["km2"] < 1.2, a            # one level-8 hexagon is 0.74 km²
        assert page.evaluate("window.studio.area.h3Cells().length") == 1
        page.mouse.click(900, 450)                                                              # the same hexagon: it goes
        page.wait_for_function("window.studio.area.area() === null", timeout=30_000)
        # a cell ID in the search box
        page.click("[data-tab=search]")
        page.fill("[data-as=search] input", "883969a40bfffff")
        page.wait_for_function("window.studio.area.area()?.from === 'h3'", timeout=30_000)
        assert "H3 cell, level 8" in page.inner_text(".bs-area [data-as=msg]")
        assert not errors, errors
        browser.close()


def test_a_buffer_a_fixes_file_and_the_time_of_each_step(studio):
    """The buffer around the area (the server grows it in DuckDB, as Studio does in the browser), the fixes file (a path that the server checks), and the build details."""
    out, url = studio
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.evaluate("(g) => window.studio.area.set(g, 'monaco-buf', 'file')", MONACO["features"][0]["geometry"])
        before = page.evaluate("window.studio.area.area().km2")
        page.wait_for_function("document.querySelector('.bs-area .where, .where') && [...document.querySelectorAll('.where')].some((e) => e.textContent.includes('monaco-buf'))")
        assert any(f"monaco-buf" in e for e in page.eval_on_selector_all(".where", "els => els.map((e) => e.textContent)")), "the card The area says where the area is saved"
        _unfold(page, 2)
        page.fill(".bs-area [data-as=buffer-field] input", "500")
        page.wait_for_function(f"window.studio.area.area().km2 > {before} * 1.5", timeout=60_000)          # 500 m around Monaco: about 2.4 -> 7 km²
        assert "+500 m" in page.inner_text("[data-as=kpis]")
        assert page.evaluate("window.studio.area.area().base.length") == 2, "the shape as chosen stays"
        page.fill(".bs-area [data-as=buffer-field] input", "0")
        page.wait_for_function(f"Math.abs(window.studio.area.area().km2 - {before}) < 0.001", timeout=60_000)
        # the fixes file: a path that is there goes into the config; one that is not says so (the card Build opens when the data is chosen)
        page.wait_for_selector("[name=bs-extract]")
        _unfold(page, 3)
        page.click("[name=bs-extract][value=monaco] + span")
        _options(page)
        page.fill("[data-ss=fixes]", str(ROOT / "tests" / "data" / "osm_fixes.yaml"))
        page.wait_for_function("document.querySelector('[data-ss=fixes-msg]').textContent === 'Found.'")
        assert 'osm_overrides: "' in page.text_content("[data-ss=yaml]")
        page.fill("[data-ss=fixes]", "/nowhere/fixes.yaml")
        page.wait_for_function("document.querySelector('[data-ss=fixes-msg]').textContent.includes('not a file')")
        assert "osm_overrides" not in page.text_content("[data-ss=yaml]")
        assert not errors, errors
        browser.close()


def test_the_folder_of_the_areas_can_change(studio, tmp_path):
    """The folder of the areas is chosen on the page: it is shown, the picker lists the folders inside, a folder that is not there is made on request, and the areas of the new folder are listed."""
    out, url = studio
    other = tmp_path / "other-areas"
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.evaluate("rs.sections.fold('areas', false)")
        assert page.inner_text(".bs-folder [data-fs=path]") == str(out)
        page.click(".bs-folder [data-fs=change]")
        page.wait_for_function("document.querySelector('.bs-folder [data-fs=input]').value.length > 0")
        assert page.input_value(".bs-folder [data-fs=input]") == str(out)
        assert out.name != "" and page.locator(".bs-folder [data-dir]").count() >= 1, "the folders inside are listed"
        # a folder that is not there: the button says that it makes it
        page.fill(".bs-folder [data-fs=input]", str(other))
        page.press(".bs-folder [data-fs=input]", "Enter")
        page.wait_for_function("document.querySelector('.bs-folder [data-fs=use]').textContent.startsWith('Create')")
        assert not other.exists()
        page.click(".bs-folder [data-fs=use]")
        page.wait_for_function(f"document.querySelector('.bs-folder [data-fs=path]').textContent === {json.dumps(str(other))}")
        assert other.is_dir() and "No area in this folder yet." in page.inner_text(".saved-area, #sb") 
        # the page says where the next area is saved
        page.evaluate("(g) => window.studio.area.set(g, 'elsewhere', 'file')", MONACO["features"][0]["geometry"])
        page.wait_for_function(f"[...document.querySelectorAll('.where')].some((e) => e.textContent.includes({json.dumps(str(other))}))")
        # back to the first folder: its areas are listed again. The list of the folder now (asked by Change…) comes late, after the typed path: the page keeps the typed path
        _unfold(page, 1)
        page.evaluate("rs.sections.fold('areas', false)")
        page.evaluate("""(() => { const f = window.fetch; window.fetch = (u, o) => String(u).endsWith('api/folders') && o?.body === ''
            ? new Promise((r) => setTimeout(r, 1500)).then(() => f(u, o)) : f(u, o); })()""")
        page.click(".bs-folder [data-fs=change]")
        page.fill(".bs-folder [data-fs=input]", str(out))
        page.press(".bs-folder [data-fs=input]", "Enter")
        page.wait_for_function("document.querySelector('.bs-folder [data-fs=use]').textContent === 'Use this folder'")
        page.wait_for_timeout(2000)                                                               # the late answer has come
        assert page.input_value(".bs-folder [data-fs=input]") == str(out)
        page.click(".bs-folder [data-fs=use]")
        page.wait_for_function(f"document.querySelector('.bs-folder [data-fs=path]').textContent === {json.dumps(str(out))}")
        assert not errors, errors
        browser.close()


def test_saved_areas_are_listed_opened_and_deleted(studio):
    """The areas are on the disk: a new visit lists them (the card Your areas). Open puts the area and the settings of one into the steps, with its result and its map
    (step 7); Delete deletes the folder of an area after the person confirms. It uses the areas of the first tests (the tests run in the order of the file)."""
    out, url = studio
    if not (out / "monaco-a" / "monaco-a.duckdb").exists():
        pytest.skip("needs the area that the first test builds")
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open(browser, url, viewport={"width": 1280, "height": 900})
        page.wait_for_selector(".saved-area")
        assert "monaco-a" in page.eval_on_selector_all(".saved-area", "els => els.map(e => e.dataset.name)")
        row = page.locator(".saved-area[data-name=monaco-a]")
        # a saved area under the mouse shows its boundary on the map (the purple outline of the search list), and it goes when the mouse leaves
        preview = "(() => { try { return window.studio.map.getSource('bs-preview').serialize().data.features.length; } catch { return -1; } })()"
        page.evaluate("window.studio.map.jumpTo({center: [0, 0], zoom: 2})")
        row.hover()
        page.wait_for_function(f"{preview} === 1", timeout=30_000)
        page.wait_for_function("window.studio.map.getZoom() > 6", timeout=30_000)      # the map moves to the area
        page.mouse.move(900, 400)
        page.wait_for_function(f"{preview} === 0")
        text = row.inner_text()
        assert "MB" in text, text
        has = row.locator(".sa-ico").evaluate_all("els => els.map((e) => e.dataset.has)")
        assert has == ["driving", "walking", "layers"], has      # monaco-a: two networks and the base-map layers (no levels, no ground, no sea)
        assert row.locator("button").all_inner_texts() == ["Open", "Delete"]
        # Open (a click on the card) shows the area on this page's map: the page does not load again
        page.evaluate("window.__same = 1")
        row.locator("strong").click()
        page.wait_for_function(OPEN, arg="monaco-a", timeout=120_000)
        assert page.evaluate("window.__same") == 1 and page.url.endswith("?area=monaco-a")
        page.wait_for_function("window.map.getStyle().layers.some((l) => l.id.startsWith('rs-'))", timeout=60_000)
        assert page.evaluate("window.studio.settings.values().modes") == ["driving", "walking"]
        assert page.evaluate("window.studio.settings.values().levels") is False and page.evaluate("window.studio.settings.values().build_features") is True
        assert page.evaluate("[...document.querySelectorAll('input[name=saved-area]:checked')].map((e) => e.value)") == ["monaco-a"]
        _unfold(page, 5)
        assert "monaco-a" in page.inner_text("[data-rs=line]") and "monaco-a.duckdb" in page.inner_text(".bs-files")
        # Delete: the person confirms, and the folder goes; the downloaded extract stays
        if (out / "monaco-dem").is_dir():
            page.once("dialog", lambda d: d.accept())
            page.locator(".saved-area[data-name=monaco-dem] [data-do=delete]").click()
            page.wait_for_function("!document.querySelector('.saved-area[data-name=monaco-dem]')")
            assert not (out / "monaco-dem").exists() and (out / "osm" / "monaco-latest.osm.pbf").is_file()
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
