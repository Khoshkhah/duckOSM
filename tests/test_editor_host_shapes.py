"""The editor on hosts that do not have the shape of `duckosm edit` (docs/design/test_host.md). The editor is a library for hosts (AGENTS.md, "Hosts of the editor and Ask"):
a host must only move its version, so the contract is tested on the shapes that Studio has. The hover bug, the road index and the missing tags column all came from this gap.
Here: array rows in the road table (`roadKeys`). The area without a tags column is in test_edit_page.py (`restored_url`)."""
from editor_fixtures import ROAD, _open, monaco, page_url, pw  # noqa: F401


def test_a_host_with_array_rows_in_the_road_table(monaco, page_url):
    """Studio's `RS_SIDE.props` rows are arrays and the host gives `roadKeys` (the column names); it has no `rsGetProps`. The editor finds the OSM way of a picked row and the rows of
    a way (its highlight sets), with no `roadIdOfRow` and no `roadRows` from the host."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context()

        def arrays(route):
            r = route.fetch()
            body = r.text().replace(
                "const panel = await initFixesPanel({",
                "const __keys = Object.keys(window.RS_SIDE.props[0]); window.__objProps = window.RS_SIDE.props; "
                "window.RS_SIDE = { ...window.RS_SIDE, props: window.__objProps.map((p) => __keys.map((k) => p[k])) }; window.rsGetProps = undefined; "
                "const panel = await initFixesPanel({ roadKeys: __keys,", 1)
            assert "roadKeys: __keys" in body, "the page of duckosm edit has changed: update this test"
            route.fulfill(response=r, body=body)

        ctx.route(page_url, arrays)
        page = ctx.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(page_url)
        page.wait_for_function("window.fixesPanel && document.body.classList.contains('editing-fixes')", timeout=60_000)
        assert page.evaluate("Array.isArray(window.RS_SIDE.props[0])"), "the rows are arrays now"
        # a click on a road of the map: the row is an array; the panel reads its OSM way from it with roadKeys
        row = page.evaluate("(id) => window.__objProps.findIndex((p) => Number(p.osm_id) === id)", ROAD)
        assert row >= 0
        page.evaluate("(r) => window.rsSelect(r)", row)
        page.wait_for_function(f"document.getElementById('fix-id').value === '{ROAD}' && document.querySelector('#fix-obj .objhead')", timeout=30_000)
        # the picked road is in the highlight set: the editor found the rows of the way in the array table
        page.wait_for_function("window.fixesPanel.drawn().lit['fix-sel'] > 0", timeout=15_000)
        assert not errors, errors
        browser.close()
