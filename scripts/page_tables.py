"""Write the digests of the tables that the duckosm-rust page builds for Monaco with a fixes file (tests/data/page_fixes/page_tables.json).

The page runs in headless Chromium (Playwright). It builds data/sample/monaco.osm.pbf in data/sample/monaco.geojson with the layers,
loads the fixes file into "Edit OSM data" and builds again, as a user does. tests/test_page_fixes.py compares a `duckosm build` with the
digests (tests/page_tables.py). Run it again when the page, the engine or the fixes file changes. docs/design/fixes_editor.md.

    python scripts/page_tables.py --www <duckosm-rust>/crates/duckosm-wasm/www

The www folder must be built (crates/duckosm-wasm/build.sh). The page loads its libraries from cdn.jsdelivr.net.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tests import page_tables as P  # noqa: E402


def built(page):
    page.wait_for_function("!document.getElementById('run').hasAttribute('aria-busy') && "
                           "!document.getElementById('step7-card').classList.contains('off')", timeout=600_000)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--www", required=True, type=Path, help="crates/duckosm-wasm/www of a built duckosm-rust checkout")
    ap.add_argument("--fixes", type=Path, default=ROOT / "tests/data/page_fixes/osm_fixes.yaml")
    ap.add_argument("--out", type=Path, default=ROOT / "tests/data/page_fixes/page_tables.json")
    a = ap.parse_args()
    sys.path.insert(0, str(a.www))
    from playwright.sync_api import sync_playwright
    from serve import start                      # the server of the page's tests (COOP and COEP headers)

    server = start(0, quiet=True, directory=a.www)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1400, "height": 900})
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(f"http://127.0.0.1:{server.server_address[1]}/index.html")
        page.wait_for_function("window.mapReady && window.timings && window.timings.duckdbStart", timeout=120_000)
        page.evaluate("localStorage.clear()")

        def q(sql):
            return page.evaluate(f"window.duckApi.query({sql!r}).then((r) => r.toArray().map((x) => x.toJSON()))")
        page.set_input_files("#file", str(ROOT / "data/sample/monaco.geojson"))
        page.wait_for_function("window.areas >= 1", timeout=60_000)
        page.click("#tab-region")
        page.set_input_files("#region", str(ROOT / "data/sample/monaco.osm.pbf"))
        page.click("#run")
        built(page)
        page.evaluate("(t) => window.fixesPanel.load(t)", a.fixes.read_text())
        n = page.evaluate("window.fixesPanel.count()")
        page.evaluate("document.getElementById('run').click()")
        page.wait_for_timeout(1500)
        built(page)
        assert not errors, errors
        res = {"page": {"commit": subprocess.run(["git", "-C", str(a.www), "rev-parse", "HEAD"], capture_output=True,
                                                                      text=True).stdout.strip(), "changes": n},
               "tables": P.digests(q)}
        browser.close()
    server.shutdown()
    a.out.write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")
    print(f"{a.out}: {len(res['tables'])} tables, {n} changes in the panel, page at {res['page']['commit'][:8]}")


if __name__ == "__main__":
    main()
