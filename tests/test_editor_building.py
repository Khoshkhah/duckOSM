"""The Building tab of the fixes editor in a browser: a click on a building, the preset fields, the walls (numbers, types, doors, selection), a building that is a relation, the
box "in your changes", and the kind of a door. The kit (fixtures, helpers) is `editor_fixtures.py`. Each test replaces a check of the end-to-end test of Studio.
Headless Chromium (Playwright); the tests skip themselves without Playwright and its browser."""
import shutil

import duckdb
import pytest

from editor_fixtures import ROAD, _QUERY, _free_port, _open_clean, _pick_road, _serve, monaco, page_url, pw  # noqa: F401

VIEW = {"viewport": {"width": 1400, "height": 900}}      # wide, so that the middle of the map is not under the panel
BIG_BUILDING = ("SELECT osm_id, ST_X(ST_PointOnSurface(geom)) AS x, ST_Y(ST_PointOnSurface(geom)) AS y FROM features.buildings WHERE osm_type = 'way' "
                "AND map_extract(tags, 'building:levels')[1] IS NULL ORDER BY ST_Area(geom) DESC LIMIT 1")


@pytest.fixture(scope="module")
def doors_url(page_url):
    """`duckosm build` makes main.building_entrances with options.building_faces, so the doors test uses the Monaco page as it is."""
    return page_url


def _rows(page, sql):
    return page.evaluate(_QUERY, sql)


def _at(page, lon, lat):
    """Move the map to the point, wait until it stands still, and give the pixel of the point on the page."""
    page.evaluate("([x, y]) => window.map.jumpTo({center: [x, y], zoom: 18})", [lon, lat])
    page.wait_for_function("!window.map.isMoving() && window.map.areTilesLoaded()", timeout=30_000)
    return page.evaluate("""([x, y]) => { const p = window.map.project([x, y]), c = window.map.getCanvas().getBoundingClientRect(); return [p.x + c.left, p.y + c.top]; }""", [lon, lat])


def _popups(page):
    return page.evaluate("document.querySelectorAll('.maplibregl-popup:not(.rs-tip)').length")


def _click_and_settle(page, px):
    """A real click, then wait until the map has had the click and the page has run its handlers (a click that must change nothing has no other sign)."""
    page.evaluate("() => { window.__clicked = false; window.map.once('click', () => { window.__clicked = true; }); }")
    page.mouse.click(*px)
    page.wait_for_function("window.__clicked === true", timeout=10_000)
    page.evaluate("() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))")
    page.wait_for_timeout(500)


def _walls(page, building_id):
    """Select the building in the panel and wait for the list of its walls and the walls on the map."""
    page.evaluate(f"window.fixesPanel.select('building', 'way', {building_id}, {{zoom: true}})")
    page.wait_for_function("document.querySelectorAll('#fix-form .wallrow').length >= 3", timeout=30_000)
    page.wait_for_function("window.map.querySourceFeatures('fix-faces').filter((f) => f.properties.n).length >= 3", timeout=30_000)
    page.wait_for_function("!window.map.isMoving()", timeout=30_000)


def _edit_it(page):
    """The object is in the changes, so the box asks first: press Edit it, so that the form opens."""
    page.wait_for_selector("#fix-inchg:not([hidden]) [data-ask=edit]", timeout=20_000)
    page.click("#fix-inchg [data-ask=edit]")
    page.wait_for_function("document.getElementById('fix-inchg').hidden", timeout=10_000)


def _add_levels(page, building_id, levels="7"):
    """Put a rule for the building in the changes (floors) through the panel."""
    page.evaluate("fixesPanel.tab('building')")
    page.evaluate(f"window.fixesPanel.select('building', 'way', {building_id}, {{zoom: true}})")
    page.wait_for_selector("#fixf-building_levels", timeout=20_000)
    page.fill("#fixf-building_levels", levels)
    page.click("#fix-save")
    page.wait_for_function("window.fixesPanel.count() === 1", timeout=10_000)


def test_a_click_on_a_building_in_the_road_or_turn_tab_picks_nothing(monaco, page_url):
    """Studio line 214: a click on a building next to a road does not pick a road in the Road tab and does not pick a junction in the Turn tab (roadstyle picks the nearest road for a
    click within a few pixels, and the panel must ignore it on a building)."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        near = _rows(page, "SELECT ST_X(ST_ClosestPoint(b.geom, e.geometry)) * 0.9 + ST_X(ST_PointOnSurface(b.geom)) * 0.1 AS x, "
                           "ST_Y(ST_ClosestPoint(b.geom, e.geometry)) * 0.9 + ST_Y(ST_PointOnSurface(b.geom)) * 0.1 AS y "
                           "FROM features.buildings b, driving.edges e WHERE b.osm_type = 'way' AND ST_Distance(b.geom, e.geometry) < 0.00003 ORDER BY ST_Area(b.geom) DESC LIMIT 1")[0]
        _pick_road(page, ROAD)
        px = _at(page, near["x"], near["y"])
        _click_and_settle(page, px)
        assert page.input_value("#fix-id") == str(ROAD), ("a click on a building picked a road in the Road tab", page.input_value("#fix-id"))
        page.click("#fix-tab-turns")
        assert page.input_value("#fix-from") == ""
        px = _at(page, near["x"], near["y"])
        _click_and_settle(page, px)
        assert page.input_value("#fix-from") == "" and page.evaluate("document.getElementById('fix-turnsec').hidden"), "a click on a building picked a junction in the Turn tab"
        assert not _popups(page)
        assert not errors, errors
        browser.close()


def test_a_building_has_floors_roof_shape_and_roof_colour(monaco, page_url):
    """Studio line 233: a real click on a building in the Building tab picks it with no popup; floors, roof shape and roof colour go into a rule, and the 3D preview of 7 floors is
    22.4 m high."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        page.click("#fix-tab-building")
        px = _at(page, b["x"], b["y"])
        page.mouse.click(*px)
        page.wait_for_function(f"document.getElementById('fix-id').value === '{b['osm_id']}' && document.querySelector('#fixf-building_levels')", timeout=20_000)
        assert _popups(page) == 0 and "Building · way" in page.inner_text("#fix-obj"), page.inner_text("#fix-obj")
        page.fill("#fixf-building_levels", "7")
        page.select_option("#fixf-roof_shape", "gabled", force=True)
        page.fill("#fixf-roof_colour", "#cc3300")
        page.click("#fix-save")
        y = page.evaluate("window.fixesPanel.text() ?? ''")
        assert "type: way" in y and "building:levels: '7'" in y and "roof:shape: gabled" in y and "roof:colour: '#cc3300'" in y, y
        page.wait_for_function("window.fixesPanel.drawn().heights.length > 0", timeout=20_000)
        assert 22.4 in page.evaluate("window.fixesPanel.drawn().heights"), page.evaluate("window.fixesPanel.drawn().heights")
        assert not errors, errors
        browser.close()


def test_wall_numbers_show_on_hover_and_a_click_on_a_wall_selects_its_row(monaco, page_url):
    """Studio line 264: the walls are drawn with a colour for each type; no wall number shows by default; a row under the pointer lights its wall and shows its one number; a wall under
    the pointer on the map lights its row; a click on the wall selects the row."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        page.click("#fix-tab-building")
        _walls(page, b["osm_id"])
        assert page.evaluate("window.fixesPanel.drawn().faces") >= 3, "the walls are drawn"
        assert page.evaluate("document.querySelectorAll('#fix-form .wallrow .sw').length === document.querySelectorAll('#fix-form .wallrow').length"), "each wall row has the colour of its type"
        labels = "window.map.queryRenderedFeatures({layers: ['fix-faces-label']}).map((f) => f.properties.n)"
        lit = "window.map.queryRenderedFeatures({layers: ['fix-faces-hover']}).map((f) => f.properties.k)"
        page.mouse.move(5, 5)
        page.wait_for_function(f"{labels}.length === 0 && {lit}.length === 0", timeout=10_000)
        row = page.locator("#fix-form .wallrow").nth(1)
        row.hover()
        page.wait_for_function(f"{labels}.length === 1", timeout=10_000)
        n, key = page.evaluate(labels)[0], row.get_attribute("data-key")
        assert f"#{n}" in row.inner_text() and set(page.evaluate(lit)) == {key}, (n, key, row.inner_text(), page.evaluate(lit))
        page.hover("#fix-tab-building")
        page.wait_for_function(f"{labels}.length === 0 && {lit}.length === 0", timeout=10_000)
        other = page.locator("#fix-form .wallrow").nth(2).get_attribute("data-key")
        mid = page.evaluate(f"""(() => {{ const f = window.map.querySourceFeatures('fix-faces').find((x) => x.properties.k === {other!r} && x.geometry.type === 'LineString');
            const c = f.geometry.coordinates; return [(c[0][0] + c[1][0]) / 2, (c[0][1] + c[1][1]) / 2]; }})()""")
        px = _at(page, *mid)
        page.mouse.move(*px)
        page.wait_for_function(f"document.querySelector('#fix-form .wallrow[data-key=\"{other}\"]').classList.contains('hover') && {labels}.length === 1", timeout=10_000)
        page.mouse.click(*px)
        page.wait_for_function(f"document.querySelector('#fix-form .wallrow[data-key=\"{other}\"]').classList.contains('sel')", timeout=10_000)
        assert not errors, errors
        browser.close()


def test_a_wall_type_waits_for_add_to_changes(monaco, page_url):
    """Studio line 273: another type for a wall is a draft until Add to changes; then it is a rule in `building_faces` with the building, and the row is marked as edited."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        page.click("#fix-tab-building")
        _walls(page, b["osm_id"])
        wall = page.evaluate("(() => { const r = document.querySelector('#fix-form .wallrow'); return [r.dataset.a, r.dataset.b, r.dataset.computed]; })()")
        new_type = "party" if wall[2] != "party" else "side"
        page.select_option("#fix-form .wallrow select", new_type, force=True)
        text = lambda: page.evaluate("window.fixesPanel.text() ?? ''")
        assert "building_faces" not in text() and not page.is_disabled("#fix-save") and "1 change not added yet" in page.inner_text("#fix-state"), "a wall type waits for Add to changes"
        page.click("#fix-save")
        y = text()
        assert "building_faces" in y and f"type: {new_type}" in y and f"building: {b['osm_id']}" in y, y
        assert "edited" in (page.get_attribute("#fix-form .wallrow", "class") or "")
        assert not errors, errors
        browser.close()


def test_a_door_on_a_wall(monaco, page_url):
    """Studio line 282: a door on a wall is a draft until Add to changes; then the rule has `door` and the type of the wall stays, a dot shows on the map, and the row still shows the door."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        page.click("#fix-tab-building")
        _walls(page, b["osm_id"])
        text = lambda: page.evaluate("window.fixesPanel.text() ?? ''")
        wall_type = page.evaluate("document.querySelector('#fix-form .wallrow').dataset.computed")
        new_type = "party" if wall_type != "party" else "side"
        page.select_option("#fix-form .wallrow select[data-wall]", new_type, force=True)
        page.click("#fix-save")
        page.select_option("#fix-form .wallrow >> nth=0 >> select[data-walldoor]", "main", force=True)
        assert "door: main" not in text() and not page.is_disabled("#fix-save"), "a door waits for Add to changes"
        page.click("#fix-save")
        y = text()
        assert "door: main" in y and f"type: {new_type}" in y, y
        page.wait_for_function("window.map.queryRenderedFeatures({layers: ['fix-faces-door']}).length === 1", timeout=20_000)
        assert page.input_value("#fix-form .wallrow >> nth=0 >> select[data-walldoor]") == "main"
        assert not errors, errors
        browser.close()


def test_a_building_that_is_a_relation_has_court_walls(monaco, page_url):
    """Studio line 290: a building that is a multipolygon relation lists the walls of its outer ring like a way, and the walls of its courtyard have the type `court`."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        rel = _rows(page, "SELECT building_id FROM main.building_faces WHERE building_type = 'relation' AND face_type = 'court' LIMIT 1")
        assert rel, "Monaco has a building with a courtyard"
        page.click("#fix-tab-building")
        page.evaluate(f"window.fixesPanel.select('building', 'relation', {rel[0]['building_id']})")
        page.wait_for_function("document.querySelectorAll('#fix-form .wallrow').length >= 5", timeout=30_000)
        assert page.evaluate("[...document.querySelectorAll('#fix-form .wallrow select[data-wall]')].some((s) => s.value === 'court')"), "the courtyard walls have the type court"
        assert not errors, errors
        browser.close()


def test_the_box_of_a_building_in_the_changes_deletes_or_edits_it(monaco, page_url):
    """Studio line 313: a building that is in the changes asks first when it is picked again. Delete it removes its records, Undo brings them back, and Edit it opens the form with the
    values of the records."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        _add_levels(page, b["osm_id"])
        text = lambda: page.evaluate("window.fixesPanel.text() ?? ''")
        pick = lambda: page.evaluate(f"window.fixesPanel.select('building', 'way', {b['osm_id']})")
        page.click("#fix-unselect")                           # the box only asks when the building is picked again
        pick()
        page.wait_for_selector("#fix-inchg:not([hidden]) [data-ask=edit]", timeout=20_000)
        box = page.inner_text("#fix-inchg")
        assert "in your changes" in box and page.is_visible("#fix-inchg [data-ask=delete]"), box
        n = page.evaluate("window.fixesPanel.count()")
        page.click("#fix-inchg [data-ask=delete]")
        page.wait_for_function(f"window.fixesPanel.count() < {n}", timeout=10_000)
        assert "building:levels: '7'" not in text(), "Delete it removes the records of the building"
        page.click("#fix-tab-changes")
        page.click("#fix-undo")
        page.wait_for_function(f"window.fixesPanel.count() === {n}", timeout=10_000)
        assert "building:levels: '7'" in text(), "Undo brings the records back"
        page.click("#fix-tab-building")
        if page.is_visible("#fix-unselect"):
            page.click("#fix-unselect")                       # the box keeps an answer while the same object stays picked
        pick()
        _edit_it(page)
        page.wait_for_function("document.querySelectorAll('#fix-form .wallrow').length >= 3", timeout=20_000)
        assert page.input_value("#fixf-building_levels") == "7", "Edit it opens the form with the values of the records"
        assert not errors, errors
        browser.close()


def test_select_each_wall(monaco, page_url):
    """Studio line 338: a wall is selected by a click on its row, by Next and Previous (they go round, and the selected row scrolls into view), and by a click on the wall on the map;
    Unselect clears the bar, the row and the outline of the wall on the map."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url, **VIEW)
        b = _rows(page, BIG_BUILDING)[0]
        page.click("#fix-tab-building")
        _walls(page, b["osm_id"])
        pos = lambda: page.inner_text("#fix-wallpos")
        assert pos().strip() == "" and page.is_disabled("#fix-wall-off"), "no wall selected: the bar is empty and Unselect is off"
        page.click("#fix-form .wallrow >> nth=1 >> .small")
        assert "Wall 2 of" in pos() and page.locator("#fix-form .wallrow.sel").count() == 1
        page.wait_for_function("!window.map.isMoving()", timeout=20_000)
        page.wait_for_function("window.map.queryRenderedFeatures({layers: ['fix-faces-sel']}).length > 0", timeout=20_000)     # the selected wall has its outline
        page.click("[data-wall-nav='1']")
        assert "Wall 3 of" in pos()
        page.click("[data-wall-nav='-1']")
        page.click("[data-wall-nav='-1']")
        assert "Wall 1 of" in pos()
        page.click("[data-wall-nav='-1']")                      # Wall 1 and Previous: the last wall; its row scrolls into view above the bar at the bottom of the panel
        page.wait_for_function("""(() => { const r = document.querySelector('#fix-form .wallrow.sel').getBoundingClientRect(), b = document.getElementById('fix-body').getBoundingClientRect(),
            bars = [...document.querySelectorAll('#fix-panel .fix-bar')].filter((x) => x.offsetParent).map((x) => x.getBoundingClientRect().top);
            return r.top >= b.top - 1 && r.bottom <= Math.min(b.bottom, ...bars) + 1; })()""", timeout=10_000)
        page.click("[data-wall-nav='1']")
        assert "Wall 1 of" in pos()
        w3 = _rows(page, f"SELECT ST_X(ST_LineInterpolatePoint(geometry, 0.5)) AS x, ST_Y(ST_LineInterpolatePoint(geometry, 0.5)) AS y FROM main.building_faces "
                         f"WHERE building_id = {b['osm_id']} AND face_index = 2")[0]
        page.mouse.click(*_at(page, w3["x"], w3["y"]))          # a real click on the third wall of the picked building
        page.wait_for_function("document.getElementById('fix-wallpos').textContent.startsWith('Wall 3 of')", timeout=20_000)
        page.click("#fix-wall-off")
        assert pos().strip() == "" and page.is_disabled("#fix-wall-off")
        page.wait_for_function("window.map.queryRenderedFeatures({layers: ['fix-faces-sel']}).length === 0", timeout=10_000)
        assert page.locator("#fix-form .wallrow.sel").count() == 0 and str(b["osm_id"]) in page.input_value("#fix-id"), "the building stays picked"
        assert not errors, errors
        browser.close()


def test_the_kind_of_a_door_is_a_node_edit(doors_url):
    """Studio line 615: the Building tab lists the doors of a building (main.building_entrances) and draws them as dots; another kind of door waits for Add to changes and then is a
    node rule with `entrance`. (The table is made by `duckosm build`.)"""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, doors_url, **VIEW)
        bid = _rows(page, "SELECT building_id FROM main.building_entrances WHERE building_id IS NOT NULL GROUP BY building_id ORDER BY count(*) DESC, building_id LIMIT 1")[0]["building_id"]
        n_doors = _rows(page, f"SELECT count(*)::INTEGER AS n FROM main.building_entrances WHERE building_id = {bid}")[0]["n"]
        door = _rows(page, f"SELECT osm_id, kind FROM main.building_entrances WHERE building_id = {bid} ORDER BY osm_id LIMIT 1")[0]
        page.click("#fix-tab-building")
        page.evaluate(f"window.fixesPanel.select('building', 'way', {bid}, {{zoom: true}})")
        page.wait_for_function("document.querySelectorAll('#fix-form .doorrow').length > 0", timeout=30_000)
        assert page.locator("#fix-form .doorrow").count() == n_doors
        page.wait_for_function("!window.map.isMoving()", timeout=30_000)
        page.wait_for_function("window.fixesPanel.drawn().doors > 0 && window.map.queryRenderedFeatures({layers: ['fix-doors-pt']}).length > 0", timeout=30_000)
        assert page.is_visible("#fix-doors-legend") and "In view" in page.inner_text("#fix-doors-info")
        new_kind = "service" if door["kind"] != "service" else "main"
        page.select_option("#fix-form .doorrow >> nth=0 >> select", new_kind, force=True)
        text = lambda: page.evaluate("window.fixesPanel.text() ?? ''")
        assert f"entrance: {new_kind}" not in text() and not page.is_disabled("#fix-save"), "a door kind waits for Add to changes"
        page.click("#fix-save")
        y = text()
        assert f"id: {door['osm_id']}" in y and "type: node" in y and f"entrance: {new_kind}" in y, y
        assert not errors, errors
        browser.close()


def test_the_build_makes_the_doors_table(monaco, page_url):
    """`duckosm build` makes main.building_entrances (the same table as Studio's page): doors with their kind, and the building whose outline has the node."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        n, on = [page.evaluate(_QUERY, q)[0]["n"] for q in ("SELECT count(*)::INTEGER AS n FROM main.building_entrances",
                                                              "SELECT count(*)::INTEGER AS n FROM main.building_entrances WHERE building_id IS NOT NULL")]
        assert n > 0 and 0 < on <= n, (n, on)
        assert not errors, errors
        browser.close()
