"""Maps and directions on the Monaco sample (docs/design/viz_on_mapstyle.md).

duckOSM owns routing (route_points, directions); mapstyle draws the pages. The map tests need the
`viz` extra (mapstyle); the page-vs-Python check also needs playwright with chromium."""
from pathlib import Path

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
    pytest.importorskip("mapstyle")
    from duckosm.route_map import write_route_map
    from duckosm.viz import render_maps
    paths = render_maps(monaco, out_dir=tmp_path)
    assert [p.name for p in paths] == ["monaco_map.html", "monaco_cycling_network.html",
                                       "monaco_driving_network.html", "monaco_walking_network.html"]
    assert all(p.stat().st_size > 100_000 for p in paths)
    assert [p.name for p in render_maps(monaco, modes=["driving"], out_dir=tmp_path, all_modes=False)] \
        == ["monaco_driving_network.html"]
    out = write_route_map(monaco, tmp_path / "planner.html", mode="driving")
    assert out.exists() and "rmSteps" in out.read_text()
    with pytest.raises(ValueError):
        render_maps(monaco, modes=["flying"], out_dir=tmp_path)
    with pytest.raises(ValueError, match="duckosm multimodal"):      # the sample build has no mm.*
        write_route_map(monaco, tmp_path / "wd.html", mode="walk+drive")


def test_planner_page_matches_duckosm(monaco, tmp_path):
    """The planner routes in the browser; its directions must equal duckOSM's directions() for the
    same edges, in every mode (both are duckOSM's: the page mirrors the Python)."""
    pytest.importorskip("mapstyle")
    sync_api = pytest.importorskip("playwright.sync_api")
    from duckosm.route_map import write_route_map
    page_file = write_route_map(monaco, tmp_path / "planner.html")
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
    pytest.importorskip("mapstyle")
    sync_api = pytest.importorskip("playwright.sync_api")
    import shutil
    from duckosm.processors.multimodal import MultimodalBuilder
    from duckosm.route_map import write_route_map
    db = tmp_path / "monaco.duckdb"
    shutil.copy(monaco, db)
    con = duckdb.connect(str(db))
    con.execute("LOAD spatial")
    MultimodalBuilder(con).run()
    con.close()
    page_file = write_route_map(db, tmp_path / "wd.html", mode="walk+drive")
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
        b.close()

