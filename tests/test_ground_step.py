"""The Ground step (web/ground_step/, docs/design/ground_step.md): the Node unit tests, the GeoTIFF that the step writes (read by rasterio), and the step in
headless Chromium (Playwright): the sources of Monaco, a file with a wrong CRS, a good file with the product, datum and licence of the catalog, the AWS fallback,
and a phone viewport."""
import functools
import http.server
import shutil
import struct
import subprocess
import threading
import zlib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
NODE = shutil.which("node")
pytestmark = pytest.mark.skipif(NODE is None, reason="needs Node")

PAGE = """<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<style>:root{--ink:#111;--mut:#666;--line:#ccc} body{margin:0;font:14px sans-serif} .rs-opt{display:flex;gap:6px} .rs-opts{display:flex;flex-direction:column}
.pick{display:flex;width:100%}</style><link rel="stylesheet" href="/web/ground_step/ground_step.css"></head>
<body><div id="sb" style="width:100%;padding:0 16px;box-sizing:border-box"><div id="g"></div></div>
<script type="module">
import { initGroundStep } from "/web/ground_step/ground_step.js";
window.area = [7.41, 43.73, 7.44, 43.75];
window.changes = 0;
window.gs = await initGroundStep({ root: document.getElementById("g"), catalog: "/src/duckosm/data/dem_sources.json", bbox: () => window.area, onChange: () => window.changes++ });
window.ready = true;
</script></body></html>"""


def _png(h):
    """A 256 x 256 Terrarium tile with the height `h` metres everywhere."""
    v = h + 32768
    row = b"\x00" + bytes([int(v // 256), int(v % 256), int((v - int(v)) * 256)]) * 256
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 256, 256, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(row * 256)) + chunk(b"IEND", b"")


@pytest.fixture(scope="module")
def server():
    class H(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/page.html":
                body = PAGE.encode()
                self.send_response(200); self.send_header("Content-Type", "text/html"); self.send_header("Content-Length", str(len(body))); self.end_headers()
                self.wfile.write(body)
            else:
                super().do_GET()
        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(H, directory=str(ROOT)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()


def test_node_unit_tests():
    r = subprocess.run([NODE, str(ROOT / "web/ground_step/ground_units.mjs")], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr + r.stdout


def test_the_geotiff_of_the_step_is_read_by_rasterio(tmp_path):
    rasterio = pytest.importorskip("rasterio")
    out = tmp_path / "t.tif"
    js = (f"import {{ writeGeoTiff }} from '{ROOT}/web/ground_step/ground_core.js'; import fs from 'node:fs';"
          "const d = Float32Array.from({length: 12}, (_, i) => i); "
          f"fs.writeFileSync('{out}', writeGeoTiff({{width: 4, height: 3, west: 7.4, north: 43.8, dx: 0.001, dy: 0.0008, data: d}}));")
    subprocess.run([NODE, "--input-type=module", "-e", js], check=True)
    with rasterio.open(out) as ds:
        assert ds.crs.to_epsg() == 4326 and (ds.width, ds.height) == (4, 3) and ds.nodata == -9999.0 and ds.dtypes == ("float32",)
        assert ds.read(1)[2, 3] == 11 and abs(ds.bounds.left - 7.4) < 1e-9 and abs(ds.bounds.top - 43.8) < 1e-9
        assert abs(ds.transform.a - 0.001) < 1e-12 and abs(ds.transform.e + 0.0008) < 1e-12


def _geotiff(tmp_path, name, epsg=4326):
    rasterio = pytest.importorskip("rasterio")
    import numpy as np
    from rasterio.transform import from_origin
    p = tmp_path / name
    with rasterio.open(p, "w", driver="GTiff", width=20, height=20, count=1, dtype="float32", crs=f"EPSG:{epsg}",
                       transform=from_origin(7.40, 43.76, 0.002, 0.001), nodata=-9999.0) as ds:
        ds.write(np.full((1, 20, 20), 50, "float32"))
    return p


def test_the_step_in_a_browser(server, tmp_path):
    pw = pytest.importorskip("playwright.sync_api")
    good, bad = _geotiff(tmp_path, "good.tif"), _geotiff(tmp_path, "bad.tif", 3006)
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 390, "height": 800})           # a phone
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://s3.amazonaws.com/elevation-tiles-prod/**", lambda r: r.fulfill(status=200, body=_png(100), content_type="image/png",
                                                                                         headers={"Access-Control-Allow-Origin": "*"}))
        page.goto(server + "/page.html")
        page.wait_for_function("window.ready")
        # Monaco: EU-DTM, FABDEM, Copernicus GLO-30 first; nothing is chosen (a file would get the label of a guess); with no choice there is no DEM
        names = page.eval_on_selector_all('[data-gs=list] input', "els => els.map(e => e.value)")
        assert names == ["eudtm", "fabdem", "copernicus-glo30", "eu-dem"]
        assert page.query_selector('input[name=gs-src]:checked') is None
        assert "Choose the source of your DEM file" in page.inner_text("[data-gs=detail]")
        assert "needs a key" in page.inner_text("[data-gs=list]")
        assert "not for commercial use" in page.inner_text("[data-gs=list]")
        assert page.evaluate("window.gs.dem()") is None
        # no horizontal scroll on a phone
        assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth")
        # a file in another CRS: the message with the command, and no DEM
        page.check('input[value=fabdem]')
        assert "Copernicus" not in page.inner_text("[data-gs=detail]") and "CC-BY-NC-SA-4.0" in page.inner_text("[data-gs=detail]")
        page.set_input_files('[data-gs=file]', str(bad))
        page.wait_for_selector(".gs-bad")
        assert "EPSG:3006" in page.inner_text(".gs-bad") and "gdalwarp -t_srs EPSG:4326 -ot Float32" in page.inner_text(".gs-bad")
        assert "EPSG:3006" in (page.evaluate("window.gs.dem().catch(e => e.message)"))
        # a good file: bytes, product, datum and licence from the catalog
        page.set_input_files('[data-gs=file]', str(good))
        page.wait_for_function("document.querySelector('[data-gs=pick] span').textContent === 'good.tif'")
        assert page.query_selector(".gs-bad") is None
        dem = page.evaluate("window.gs.dem().then(d => ({...d, bytes: d.bytes.byteLength}))")
        assert dem["name"] == "good.tif" and dem["product"] == "DTM" and dem["vertical_datum"] == "EGM2008" and dem["license"] == "CC-BY-NC-SA-4.0"
        assert dem["not_reference"] is False and dem["bytes"] == good.stat().st_size
        # Copernicus GLO-30 is a DSM
        page.check('input[value=copernicus-glo30]')
        page.set_input_files('[data-gs=file]', str(good))
        page.wait_for_function("document.querySelector('[data-gs=pick] span').textContent === 'good.tif'")
        assert page.evaluate("window.gs.dem().then(d => d.product)") == "DSM"
        # the fallback: no file, a DEM from the tiles, labelled and not a reference
        page.check('input[value=aws-terrain-tiles]')
        assert "Ground: AWS Terrain Tiles" in page.inner_text("[data-gs=aws]")
        dem = page.evaluate("window.gs.dem().then(d => ({...d, bytes: Array.from(new Uint8Array(d.bytes.slice(0, 4)))}))")
        assert dem["name"] == "Ground: AWS Terrain Tiles" and dem["product"] == "DSM" and dem["not_reference"] is True and dem["bytes"] == [0x49, 0x49, 42, 0]
        # a new area gives a new list: Tartu starts with the Estonian DTM, and nothing is chosen again
        page.evaluate("window.area = [26.60, 58.30, 26.85, 58.45]; window.gs.refresh()")
        assert page.query_selector('input[name=gs-src]:checked') is None and page.evaluate("window.gs.dem()") is None
        assert page.eval_on_selector_all('[data-gs=list] input', "els => els[0].value") == "ee-maaamet-dtm"
        page.check('input[value=ee-maaamet-dtm]')
        assert "EPSG:3301" in page.inner_text("[data-gs=detail]")
        assert page.evaluate("document.documentElement.scrollWidth <= window.innerWidth")
        assert errors == []
        browser.close()
