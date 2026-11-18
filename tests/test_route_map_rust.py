"""`duckosm route-map` and `viz` in a browser, on Monaco (group F of docs/design/all_in_rust.md).

The planner page routes with the program's router built as WebAssembly. Here the page's route from A to B, in each mode, has the
edges of `duckosm route-points` for the same points, and the page's steps are the texts of `duckosm directions`. The
walk + drive page opens on Walk + drive, with a change of mode, and its Fastest and Shortest each win on their own measure. At
390 x 844 (a phone) the pages load, the planner draws its route and nothing is wider than the screen.

Needs the program (rust/target/release/duckosm, or DUCKOSM_RUST) built with the wasm32 target, and playwright with chromium."""
import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
EXE = Path(os.environ.get("DUCKOSM_RUST", ROOT / "rust" / "target" / "release" / "duckosm"))
sync_api = pytest.importorskip("playwright.sync_api")
if not EXE.exists():
    pytest.skip(f"no {EXE}: cargo build --release -p duckosm-cli", allow_module_level=True)


def run(*args, cwd=None):
    return subprocess.run([str(EXE), *map(str, args)], check=True, capture_output=True, text=True, cwd=cwd).stdout


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    d = tmp_path_factory.mktemp("monaco")
    s = ROOT / "data" / "sample"
    (d / "config.yaml").write_text(
        f"name: monaco\noutput_path: {d}\nsource: {{type: pbf, pbf_path: {s / 'monaco.osm.pbf'}}}\n"
        f"boundary: {{path: {s / 'monaco.geojson'}}}\nmodes: [driving, walking, cycling]\n"
        f"osm_overrides: {ROOT / 'osm_overrides' / 'osm_overrides.yaml'}\nmultimodal: {{enabled: true}}\n")
    run("--config", d / "config.yaml", cwd=d)
    return d / "monaco.duckdb"


@pytest.fixture(scope="module")
def browser():
    with sync_api.sync_playwright() as p:
        try:
            b = p.chromium.launch()
        except sync_api.Error as e:
            pytest.skip(f"playwright browser missing: {e}")
        yield b
        b.close()


def open_page(browser, path, width=1300, height=850):
    pg = browser.new_page(viewport={"width": width, "height": height})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(path.resolve().as_uri())
    return pg, errors


def test_planner_routes_as_the_program(monaco, browser, tmp_path):
    page = tmp_path / "planner.html"
    run("route-map", monaco, "-o", page)
    pg, errors = open_page(browser, page)
    pg.wait_for_function("() => window.rmSteps && window.rmSteps.length > 1", timeout=60000)
    for label, mode in (("Drive", "driving"), ("Walk", "walking"), ("Cycle", "cycling")):
        pg.locator("#rm-modes").get_by_role("button", name=label, exact=True).click()
        pg.wait_for_function(f"() => window.rmLast && window.rmLast.modes.join() === '{mode}' && window.rmLast.res", timeout=30000)
        res = pg.evaluate("() => window.rmLast.res")
        route = res["route"]                                    # its edge ids are rounded by JavaScript: res.edge_ids has them
        a, b = route["start"]["point"], route["end"]["point"]
        want = json.loads(run("route-points", monaco, *a, *b, "-m", mode))
        assert [int(e) for e in res["edge_ids"]] == want["edges"], label
        assert route["time_s"] == pytest.approx(want["time_s"], abs=1e-9), label
        steps = json.loads(subprocess.run([str(EXE), "directions", str(monaco), "-", "-m", mode], input=json.dumps(want),
                                          check=True, capture_output=True, text=True).stdout)
        assert pg.evaluate("() => window.rmSteps") == [s["text"] for s in steps], label
        # the panel's list of edges is the route's
        ids = [int(x) for x in pg.evaluate("() => [...document.querySelectorAll('.rm-edges code')].map(c => c.textContent)")]
        assert ids == want["edges"], label
    assert not errors, errors


def test_walk_drive_page(monaco, browser, tmp_path):
    page = tmp_path / "wd.html"
    run("route-map", monaco, "-m", "walk+drive", "-o", page)
    pg, errors = open_page(browser, page)
    pg.wait_for_function("() => window.rmSteps && window.rmSteps.length > 1", timeout=60000)
    assert pg.evaluate("() => document.getElementById('rm-mode').selectedOptions[0].textContent") == "Walk + drive"
    assert "change" in pg.evaluate("() => document.getElementById('rm-result').innerText")      # changes of mode
    res = pg.evaluate("() => window.rmLast.res")
    a, b = res["route"]["start"]["point"], res["route"]["end"]["point"]
    want = json.loads(run("route-multimodal-points", monaco, *a, *b, "--modes", "walking,driving"))
    assert [int(e) for e in res["edge_ids"]] == [e for _, e in want["edges"]]
    fast = pg.evaluate("() => window.rmLast.res")                # Fastest and Shortest: each wins on its own measure
    pg.click("#rm-weight button[data-w=length]")
    short = pg.evaluate("() => window.rmLast.res")
    assert short["len"] < fast["len"] and fast["time"] <= short["time"], (fast["len"], short["len"], fast["time"], short["time"])
    assert not errors, errors


def test_pages_on_a_phone(monaco, browser, tmp_path):
    run("viz", monaco, "-m", "driving", "--out-dir", tmp_path)
    run("route-map", monaco, "-o", tmp_path / "planner.html")
    for name in ("monaco_driving_network.html", "planner.html"):
        pg, errors = open_page(browser, tmp_path / name, 390, 844)
        pg.wait_for_function("() => window.map && window.map.isStyleLoaded()", timeout=60000)
        if name == "planner.html":
            pg.wait_for_function("() => window.rmSteps && window.rmSteps.length > 1", timeout=60000)
            box = pg.locator("#rm-panel").bounding_box()
            assert box["width"] <= 390 and box["y"] > 300, box             # the panel is a sheet at the bottom
            assert pg.locator("#map").bounding_box()["height"] <= box["y"] + 1   # the map is above the sheet, not under it
        assert pg.evaluate("() => document.documentElement.scrollWidth") <= 390, name
        assert not errors, (name, errors)
