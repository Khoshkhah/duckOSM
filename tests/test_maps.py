"""Maps and directions on the Monaco sample (docs/design/viz_on_mapstyle.md).

duckOSM owns routing (route_points, directions); the Rust program writes the pages (`duckosm viz`, `duckosm route-map`).
The page-vs-SDK checks need playwright with chromium."""
from pathlib import Path

from cli_run import invoke
import duckdb
import pytest

from duckosm import Config, DuckOSM, directions, route_points

ROOT = Path(__file__).resolve().parents[1]
A, B = (7.4155, 43.7285), (7.4400, 43.7480)            # Fontvieille -> Larvotto


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    d = tmp_path_factory.mktemp("monaco")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    if cfg.osm_overrides:
        cfg.osm_overrides = str(ROOT / cfg.osm_overrides)     # the sample's fixes file is relative to the repo too
    cfg.output_path = str(d)
    import os
    cwd = os.getcwd(); os.chdir(d)
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(cwd)
    return d / "monaco.duckdb"


def test_directions_follow_the_route(monaco):
    con = duckdb.connect(str(monaco), read_only=True)
    con.execute("LOAD spatial")
    r = route_points(con, A, B, radius_m=300)            # Larvotto: the beach, ~100 m from a road
    steps = directions(con, r)
    assert steps[0]["type"] == "depart" and steps[0]["text"].startswith("Head ")
    assert steps[-1]["text"] == "Arrive at your destination"
    assert any(s["type"] == "roundabout" and " exit" in s["text"] for s in steps)
    assert abs(sum(s["length_m"] for s in steps) - (r["length_m"] - r["start"]["access_m"]
                                                     - r["end"]["access_m"])) < 1      # every metre once
    assert directions(con, None) == []


def test_viz_and_route_map_pages(monaco, tmp_path):
    r = invoke(["viz", str(monaco), "--out-dir", str(tmp_path)])
    assert r.exit_code == 0, r.output
    paths = sorted(tmp_path.glob("*.html"))
    assert [p.name for p in paths] == ["monaco_cycling_network.html", "monaco_driving_network.html",
                                       "monaco_map.html", "monaco_walking_network.html"]
    assert all(p.stat().st_size > 100_000 for p in paths)
    one = tmp_path / "one"
    assert invoke(["viz", str(monaco), "-m", "driving", "--out-dir", str(one)]).exit_code == 0
    assert [p.name for p in one.glob("*.html")] == ["monaco_driving_network.html"]
    out = tmp_path / "planner.html"
    r = invoke(["route-map", str(monaco), "-m", "driving", "-o", str(out)])
    assert r.exit_code == 0, r.output
    assert out.exists() and "rmSteps" in out.read_text()
    assert invoke(["viz", str(monaco), "-m", "flying", "--out-dir", str(tmp_path)]).exit_code != 0
    r = invoke(["route-map", str(monaco), "-m", "walk+drive", "-o", str(tmp_path / "wd.html")])
    assert r.exit_code != 0 and "duckosm multimodal" in r.output      # the sample build has no mm.*


def test_planner_page_matches_duckosm(monaco, tmp_path):
    """The planner routes in the browser; its directions must equal duckOSM's directions() for the
    same edges, in every mode (both are duckOSM's: the page mirrors the Rust router)."""
    sync_api = pytest.importorskip("playwright.sync_api")
    page_file = tmp_path / "planner.html"
    assert invoke(["route-map", str(monaco), "-o", str(page_file)]).exit_code == 0
    con = duckdb.connect(str(monaco), read_only=True)
    con.execute("LOAD spatial")
    with sync_api.sync_playwright() as p:
        try:
            b = p.chromium.launch()
        except sync_api.Error as e:                      # no chromium installed
            pytest.skip(f"playwright browser missing: {e}")
        pg = b.new_page(viewport={"width": 1300, "height": 850})
        pg.goto(page_file.resolve().as_uri())
        pg.wait_for_function("() => window.rmSteps && window.rmSteps.length > 1", timeout=30000)
        for label, mode in (("Drive", "driving"), ("Walk", "walking"), ("Cycle", "cycling")):
            pg.locator("select:has(option:text('Drive'))").select_option(label=label)
            pg.wait_for_timeout(1500)
            ids = [int(x) for x in pg.evaluate(
                "() => [...document.querySelectorAll('.rm-edges code')].map(c => c.textContent)")]
            assert ids, label
            assert pg.evaluate("() => window.rmSteps") == \
                [s["text"] for s in directions(con, {"edges": ids}, mode=mode)], label
        b.close()


def test_route_map_opens_on_walk_drive(monaco, tmp_path):
    """`route-map -m walk+drive` (the docs' planner): every mode in front, Walk + drive chosen."""
    sync_api = pytest.importorskip("playwright.sync_api")
    import shutil
    db = tmp_path / "monaco.duckdb"
    shutil.copy(monaco, db)
    assert invoke(["multimodal", str(db)]).exit_code == 0
    page_file = tmp_path / "wd.html"
    assert invoke(["route-map", str(db), "-m", "walk+drive", "-o", str(page_file)]).exit_code == 0
    with sync_api.sync_playwright() as p:
        try:
            b = p.chromium.launch()
        except sync_api.Error as e:
            pytest.skip(f"playwright browser missing: {e}")
        pg = b.new_page(viewport={"width": 1300, "height": 850})
        pg.goto(page_file.resolve().as_uri())
        pg.wait_for_function("() => window.rmSteps && window.rmSteps.length > 1", timeout=30000)
        assert pg.evaluate("() => document.getElementById('rm-mode').selectedOptions[0].textContent") == "Walk + drive"
        assert "change" in pg.evaluate("() => document.getElementById('rm-result').innerText")   # changes of mode
        fast = pg.evaluate("() => window.rmLast.res")          # Fastest and Shortest: each wins on its own measure
        pg.click("#rm-weight button[data-w=length]")
        short = pg.evaluate("() => window.rmLast.res")
        assert short["len"] < fast["len"] and fast["time"] <= short["time"], (fast["len"], short["len"], fast["time"], short["time"])
        b.close()



def test_viz_page_follows_the_google_key(monaco, tmp_path, monkeypatch):
    """The key of GOOGLE_MAPS_KEY goes into the page options (Street View and Google 3D); no key: an empty one.
    Note: the Google 3D switch (`--google-3d`) changes no byte of the page in this roadstyle build, so it is not checked."""
    def page(*flags):
        assert invoke(["viz", str(monaco), "-m", "driving", "--out-dir", str(tmp_path), *flags]).exit_code == 0
        return (tmp_path / "monaco_driving_network.html").read_text()
    monkeypatch.delenv("GOOGLE_MAPS_KEY", raising=False)
    assert '"street_view_key":""' in page()
    monkeypatch.setenv("GOOGLE_MAPS_KEY", "AIza-test-key")        # invoke passes the environment on
    assert '"street_view_key":"AIza-test-key"' in page()
    assert invoke(["viz", str(monaco), "-m", "driving", "--out-dir", str(tmp_path), "--no-google-3d"]).exit_code == 0
