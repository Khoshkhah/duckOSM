"""The Turn tab, the Changes tab, New road and Delete of the fixes editor (`duckosm edit`, web/fixes_editor/fixes_panel.js), in headless Chromium (Playwright).
These checks were Studio's end-to-end test of the editor (studio_fixes_e2e.py). They are here so that the editor is tested where it lives. The junction dots and
the colours of the Turn tab are in test_edit_page.py. Each test starts with an empty fixes file (`_open_clean`)."""
import shutil

import duckdb
import pytest

from editor_fixtures import ROAD, _LIT, _QUERY, _open_clean, _pick_road, _serve, monaco, page_url, pw  # noqa: F401

DELETED = "#dc2626"                       # the colour of a deleted object (the set fix-gone)
ARMS = ("(SELECT nid FROM (SELECT source AS nid, target AS o FROM driving.edges UNION ALL SELECT target, source FROM driving.edges) GROUP BY nid "
        "HAVING count(DISTINCT o) >= 3)")


def _rows(page, sql):
    return page.evaluate(_QUERY, sql)


def _text(page):
    """The fixes file as the panel has it now ('' when there is no change)."""
    return page.evaluate("window.fixesPanel.text() ?? ''")


def _load(page, text):
    """Put changes into the panel (the file text), as if the user had made them."""
    page.evaluate("(t) => window.fixesPanel.load(t)", text)


def _settled(page):
    page.wait_for_function("window.map.loaded() && !window.map.isMoving() && window.map.areTilesLoaded()", timeout=30_000)


def _pixel(page, lon, lat):
    """Move the map to the point and give the pixel of the point on the page."""
    page.evaluate("([x, y]) => window.map.jumpTo({center: [x, y], zoom: 18})", [lon, lat])
    _settled(page)
    return page.evaluate("""([x, y]) => { const p = window.map.project([x, y]), c = window.map.getCanvas().getBoundingClientRect(); return [p.x + c.left, p.y + c.top]; }""", [lon, lat])


def _edit_it(page, box="#fix-inchg"):
    """The object of the tab is in the changes, so a box asks first: press Edit it, so that the form opens."""
    page.wait_for_selector(f"{box} [data-ask=edit]", timeout=15_000)
    page.click(f"{box} [data-ask=edit]")
    page.wait_for_function(f"document.querySelector('{box}').hidden", timeout=10_000)


def _turn_junction(page):
    """A road that arrives at a junction where two other roads meet (a road to start a turn from)."""
    return int(_rows(page, "SELECT a.osm_id FROM driving.edges a JOIN driving.edges b ON b.source = a.target AND b.osm_id != a.osm_id "
                           "JOIN driving.edges c ON c.target = a.target AND c.osm_id NOT IN (a.osm_id, b.osm_id) LIMIT 1")[0]["osm_id"])


def test_the_turn_tab_junction_list_a_dead_end_and_an_arriving_road(monaco, page_url):
    """Replaces studio_fixes_e2e.py 391-435. The junction list says how many roads leave each junction; a road that arrives can be the start road (a click on it); and
    a road that ends in a dead end gets the reason "No driving road leaves this junction" with the roads that arrive."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        frm = _turn_junction(page)
        page.evaluate("fixesPanel.tab('turns')")
        page.fill("#fix-from", str(frm))
        page.click("#fix-turn-find")
        page.wait_for_selector("#fix-out .turn", timeout=30_000)
        # the junction list: one option for each junction, and the chosen one says how many roads leave it
        assert "leave" in page.eval_on_selector("#fix-via", "(s) => s.selectedOptions[0].textContent"), "the junction list has the number of roads that leave"
        # a road that arrives at the junction: a click on it starts the turns from that road
        arr = page.get_attribute("#fix-out [data-from]", "data-from")
        assert arr and int(arr) != frm
        page.click("#fix-out [data-from]")
        page.wait_for_function(f"document.getElementById('fix-from').value === '{arr}'", timeout=10_000)
        # a road with no road that leaves at its ends: the tab says why, and still lists the roads that arrive
        dead = _rows(page, "SELECT e.osm_id FROM driving.edges e GROUP BY e.osm_id HAVING bool_and(NOT EXISTS "
                           "(SELECT 1 FROM driving.edges o WHERE o.source = e.target AND o.osm_id != e.osm_id)) LIMIT 1")
        assert dead, "the Monaco sample has a road that ends in a dead end"
        page.fill("#fix-from", str(dead[0]["osm_id"]))
        page.click("#fix-turn-find")
        page.wait_for_function("document.getElementById('fix-out').textContent.includes('No driving road leaves this junction')", timeout=15_000)
        assert page.locator("#fix-out .turn").count() == 0
        assert "no road leaves it" in page.eval_on_selector("#fix-via", "(s) => s.selectedOptions[0].textContent")
        # and the first road gives its turns again
        page.fill("#fix-from", str(frm))
        page.click("#fix-turn-find")
        page.wait_for_selector("#fix-out .turn", timeout=15_000)
        assert not errors, errors
        browser.close()


def test_a_click_on_a_change_opens_the_road_the_building_the_node_and_the_turn(monaco, page_url):
    """Replaces studio_fixes_e2e.py 482. A click on a change in the Changes tab opens its object in the tab of its kind: a road in the Road tab, a building in the Building
    tab, a node in the Node tab, and a turn rule in the Turn tab (with its road and junction)."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        frm = _turn_junction(page)
        via, to = (int(v) for v in _rows(page, f"SELECT a.target AS via, b.osm_id AS to FROM driving.edges a JOIN driving.edges b ON b.source = a.target AND b.osm_id != a.osm_id "
                                              f"WHERE a.osm_id = {frm} LIMIT 1")[0].values())
        bid = int(_rows(page, "SELECT osm_id AS id FROM features.buildings WHERE osm_type = 'way' ORDER BY osm_id LIMIT 1")[0]["id"])
        nid = int(_rows(page, "SELECT osm_id AS id FROM features.traffic WHERE kind = 'traffic_signals' ORDER BY osm_id LIMIT 1")[0]["id"])
        _load(page, f"""overrides:
  - {{osm_id: {ROAD}, lanes: 3}}
extra:
  tag_edits:
    - {{type: way, id: {ROAD}, set: {{sidewalk: both}}}}
    - {{type: way, id: {bid}, set: {{building:levels: '7'}}}}
    - {{type: node, id: {nid}, set: {{traffic_signals:direction: backward}}}}
turn_restrictions:
  - {{from_way: {frm}, via_node: {via}, to_way: {to}, restriction: no_straight_on}}
""")
        assert page.evaluate("window.fixesPanel.count()") == 4
        page.evaluate("fixesPanel.tab('changes')")
        page.wait_for_function("document.querySelectorAll('#fix-list .chg').length === 4", timeout=15_000)
        for sel, tab, check in [(f".chg[data-id='{ROAD}']", "road", f"document.getElementById('fix-id').value === '{ROAD}' && document.querySelector('#fixf-lanes')"),
                                (".chg[data-kind=building]:not([data-face])", "building", f"document.getElementById('fix-id').value === '{bid}' && document.querySelector('#fixf-building_levels')"),
                                (".chg[data-type=node]", "node", f"document.getElementById('fix-id').value === '{nid}' && document.querySelector('#fixf-traffic_signals_direction')"),
                                (".chg[data-turn]", "turns", f"document.getElementById('fix-from').value === '{frm}' && document.querySelector('#fix-out .turn')")]:
            page.click("#fix-tab-changes")
            page.click(f"#fix-list {sel}")
            page.wait_for_function(check, timeout=20_000)
            assert page.get_attribute(f"#fix-tab-{tab}", "aria-selected") == "true", tab
        assert not errors, errors
        browser.close()


def test_a_new_road_is_drawn_with_two_points_and_opens_again_for_edit(monaco, page_url):
    """Replaces studio_fixes_e2e.py 515 and 528. A new road of two points (the first on a junction, so it joins the network) is in new_ways with its tags. A click on it in the
    Changes tab opens it in the New road tab: the name can change, and the points stay."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        j = _rows(page, "SELECT node_id AS id, ST_X(geom) AS x, ST_Y(geom) AS y FROM driving.nodes WHERE node_id IN (SELECT node_id FROM driving.way_nodes) ORDER BY node_id LIMIT 1")[0]
        page.evaluate("fixesPanel.tab('new')")
        page.click("#fix-draw")
        assert page.is_visible("#fix-drawbox")
        x, y = _pixel(page, j["x"], j["y"])
        page.mouse.click(x, y)
        page.wait_for_function("window.fixesPanel.drawn().draw === 1", timeout=20_000)
        assert "joins a junction" in page.inner_text("#fix-draw-n"), page.inner_text("#fix-draw-n")
        page.mouse.click(x + 150, y - 80)
        page.wait_for_function("window.fixesPanel.drawn().draw === 2", timeout=20_000)
        page.select_option("#fixn-highway", "service", force=True)      # roadstyle's select box hides the native one
        page.fill("#fixn-name", "Test Lane")
        page.click("#fix-draw-done")
        page.wait_for_function("window.fixesPanel.count() === 1", timeout=10_000)
        text = _text(page)
        assert "new_ways" in text and "id: 9000000000000" in text and "highway: service" in text and "name: Test Lane" in text, text
        assert not page.is_visible("#fix-drawbox") and page.inner_text("#fix-n") == "1"
        # a click on the new road in the Changes tab opens it for edit; the box asks first
        page.click("#fix-tab-changes")
        page.click("#fix-list .chg[data-newway]")
        _edit_it(page, "#fix-new-inchg")
        assert page.get_attribute("#fix-tab-new", "aria-selected") == "true" and page.is_visible("#fix-drawbox")
        assert page.input_value("#fixn-name") == "Test Lane"
        assert page.inner_text("#fix-draw-done") == "Update change" and page.is_visible("#fix-new-del") and not page.is_visible("#fix-draw-undo")
        page.fill("#fixn-name", "Test Lane 2")
        page.click("#fix-draw-done")
        page.wait_for_function("window.fixesPanel.text().includes('name: Test Lane 2')", timeout=10_000)
        text = _text(page)
        assert "name: Test Lane\n" not in text and page.inner_text("#fix-n") == "1", text
        assert text.count("id: 9000000000000") == 1 and text.count("- 7.") + text.count("- 43.") >= 2, "the points stayed"
        assert not page.is_visible("#fix-drawbox")
        assert not errors, errors
        browser.close()


def test_out_and_back_the_changes_and_the_road_of_the_tab_stay(monaco, page_url):
    """Replaces studio_fixes_e2e.py 589. After the panel is closed and opened again, and after `rebuilt()` (the host calls it after a build), the Changes tab lists all
    the changes and the Road tab still has its road with its saved value."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        bid = int(_rows(page, "SELECT osm_id AS id FROM features.buildings WHERE osm_type = 'way' ORDER BY osm_id LIMIT 1")[0]["id"])
        nid = int(_rows(page, "SELECT osm_id AS id FROM features.traffic WHERE kind = 'traffic_signals' ORDER BY osm_id LIMIT 1")[0]["id"])
        other = int(_rows(page, f"SELECT osm_id FROM driving.edges WHERE osm_id != {ROAD} ORDER BY osm_id LIMIT 1")[0]["osm_id"])
        _load(page, f"""overrides:
  - {{osm_id: {ROAD}, lanes: 7}}
  - {{osm_id: {other}, lanes: 2}}
extra:
  tag_edits:
    - {{type: way, id: {ROAD}, set: {{sidewalk: both}}}}
    - {{type: way, id: {bid}, set: {{building:levels: '7'}}}}
    - {{type: node, id: {nid}, set: {{traffic_signals:direction: backward}}}}
new_ways:
  - {{id: 9000000000000, nodes: [21911886, [7.4235072, 43.7371133], 21914340], tags: {{highway: service}}}}
""")
        n = page.evaluate("window.fixesPanel.count()")
        assert n == 5, n
        _pick_road(page, ROAD)
        _edit_it(page)
        assert page.input_value("#fixf-lanes") == "7"
        for how in ("close and open", "rebuilt"):
            if how == "rebuilt":
                page.evaluate("window.fixesPanel.rebuilt()")
            else:
                page.evaluate("window.fixesPanel.close()")
                page.wait_for_function("!document.body.classList.contains('editing-fixes')", timeout=10_000)
                page.evaluate("window.fixesPanel.open()")
                page.wait_for_function("document.body.classList.contains('editing-fixes')", timeout=10_000)
            assert page.inner_text("#fix-n") == str(n), how
            page.click("#fix-tab-road")
            page.wait_for_function(f"document.getElementById('fix-id').value === '{ROAD}' && document.querySelector('#fixf-lanes, #fix-inchg:not([hidden])')", timeout=20_000)
            page.click("#fix-tab-changes")
            page.wait_for_function(f"document.querySelectorAll('#fix-list .chg').length === {n}", timeout=15_000)
        assert not errors, errors
        browser.close()


def test_delete_a_building_and_a_road_they_are_in_the_changes_as_deleted_and_red(monaco, page_url):
    """Replaces studio_fixes_e2e.py 674. The Delete button of a building and of a road writes the rule `remove`. The Changes tab lists both as deleted from the data,
    and the map draws them red: the road in the highlight set fix-gone, the building in the editor's own layer."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        ob = int(_rows(page, "SELECT osm_id AS id FROM features.buildings WHERE osm_type = 'way' ORDER BY osm_id LIMIT 1")[0]["id"])
        orr = int(_rows(page, f"SELECT osm_id FROM driving.edges WHERE osm_id != {ROAD} ORDER BY length_m, osm_id LIMIT 1")[0]["osm_id"])
        # the building
        page.evaluate(f"fixesPanel.select('building', 'way', {ob}, {{zoom: true}})")
        page.wait_for_selector("#fix-delete", timeout=20_000)
        page.click("#fix-delete")
        assert "To be deleted" in page.inner_text(".fsec.danger") and page.is_visible("#fix-delete"), page.inner_text(".fsec.danger")
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 1", timeout=10_000)
        text = _text(page)
        assert f"osm_id: {ob}" in text and "remove: true" in text, text
        page.wait_for_function("window.map.queryRenderedFeatures({layers: ['fix-edits-fill', 'fix-edits-line']}).some((f) => f.properties.d === 1)", timeout=20_000)
        # the road
        page.evaluate(f"fixesPanel.select('road', 'way', {orr}, {{zoom: true}})")
        page.wait_for_selector("#fix-delete", timeout=20_000)
        page.click("#fix-delete")
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 2", timeout=10_000)
        assert f"osm_id: {orr}" in _text(page)
        page.evaluate("fixesPanel.tab('changes')")
        page.wait_for_function("document.querySelectorAll('#fix-list .chg.del').length === 2", timeout=20_000)    # the list is drawn after its SQL (the kind of each way)
        assert page.inner_text("#fix-list").count("deleted from the data") == 2
        # the road is red on the map's own road rows
        page.wait_for_function(f"({_LIT})({orr}).length > 0 && ({_LIT})({orr}).every((c) => c === '{DELETED}')", timeout=20_000)
        assert not errors, errors
        browser.close()


@pytest.fixture(scope="module")
def build_url(monaco, tmp_path_factory):
    """A server on its own copy of the Monaco database: Build again changes the file, and the other tests of this module must not see that."""
    d = tmp_path_factory.mktemp("build")
    db = d / "monaco.duckdb"
    shutil.copy(monaco, db)
    proc, url = _serve(db)
    yield db, url
    proc.terminate()
    proc.wait(10)


def test_a_traffic_signal_and_the_other_changes_are_in_the_built_database(build_url):
    """Replaces studio_fixes_e2e.py 693 and 694. A traffic signal is added on a junction (a gray dot on the map, a click on it, Kind Traffic signals, Add to changes), next
    to a road with lanes and a sidewalk, a building with floors and a roof, and a node with tags. After Build again, the signal is in features.traffic, and every other value is in the
    tables."""
    db, url = build_url
    # the data before the build: a junction with 3 roads or more, no traffic tag near it, a building with no floors, a traffic signal to tag
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    j = con.execute("SELECT n.node_id AS id, ST_X(n.geom), ST_Y(n.geom) FROM driving.nodes n WHERE n.node_id IN " + ARMS + " AND n.node_id IN (SELECT node_id FROM driving.way_nodes) "
                    "AND NOT EXISTS (SELECT 1 FROM driving.edges e WHERE (e.source = n.node_id OR e.target = n.node_id) AND e.highway IN ('motorway', 'motorway_link', 'trunk', 'trunk_link')) "
                    "AND NOT EXISTS (SELECT 1 FROM features.traffic t WHERE abs(ST_X(t.geom) - ST_X(n.geom)) < 0.0004 AND abs(ST_Y(t.geom) - ST_Y(n.geom)) < 0.0004) ORDER BY n.node_id LIMIT 1").fetchone()
    bid = con.execute("SELECT osm_id FROM features.buildings WHERE osm_type = 'way' AND map_extract(tags, 'building:levels')[1] IS NULL ORDER BY ST_Area(geom) DESC LIMIT 1").fetchone()[0]
    nid = con.execute("SELECT osm_id FROM features.traffic WHERE kind = 'traffic_signals' ORDER BY osm_id LIMIT 1").fetchone()[0]
    con.close()
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, url)
        # a road: lanes and a sidewalk (a tag edit)
        _pick_road(page, ROAD)
        page.fill("#fixf-lanes", "7")
        page.select_option("#fixf-sidewalk", "both", force=True)
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 1", timeout=10_000)
        # a building: floors and a roof
        page.evaluate(f"fixesPanel.select('building', 'way', {bid}, {{zoom: true}})")
        page.wait_for_selector("#fixf-building_levels", timeout=20_000)
        page.fill("#fixf-building_levels", "7")
        page.select_option("#fixf-roof_shape", "gabled", force=True)
        page.fill("#fixf-roof_colour", "#cc3300")
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 2", timeout=10_000)
        # a node that is a traffic signal: a direction and an added tag
        page.evaluate(f"fixesPanel.select('node', 'node', {nid})")
        page.wait_for_selector("#fixf-traffic_signals_direction", state="attached", timeout=20_000)
        page.select_option("#fixf-traffic_signals_direction", "backward", force=True)
        page.fill("#fix-addkey", "note:test")
        page.fill("#fix-addval", "hello")
        page.click("#fix-add")
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 3", timeout=10_000)
        # a traffic signal on a junction: a gray dot, a click on it
        page.evaluate("fixesPanel.tab('node')")
        x, y = _pixel(page, j[1], j[2])
        page.wait_for_function(f"window.map.queryRenderedFeatures({{layers: ['fix-nodes-pt']}}).some((f) => f.properties.k === 'junction' && f.properties.id === {j[0]})", timeout=20_000)
        page.mouse.click(x, y)
        page.wait_for_function(f"document.getElementById('fix-id').value === '{j[0]}' && document.getElementById('fix-obj').innerText.includes('Node {j[0]}') "
                               "&& !document.getElementById('fix-obj').classList.contains('loading') && document.querySelector('#fixf-highway')", timeout=20_000)
        assert "no traffic tag" in page.inner_text("#fix-obj"), page.inner_text("#fix-obj")
        page.select_option("#fixf-highway", "traffic_signals", force=True)
        assert page.input_value("#fixf-highway") == "traffic_signals"
        page.click("#fix-save")
        page.wait_for_function("window.fixesPanel.count() === 4", timeout=10_000)
        text = _text(page)
        assert f"id: {j[0]}" in text and "highway: traffic_signals" in text, text
        # Build again: the page loads again with the new tables
        page.click("#fix-tab-changes")
        assert page.is_enabled("#fix-apply"), page.inner_text("#fix-apply-why")
        with page.expect_navigation(timeout=600_000):
            page.click("#fix-apply")
        page.wait_for_function("window.fixesPanel", timeout=60_000)
        assert page.evaluate("window.fixesPanel.count()") == 4, "the changes come back from the file"
        assert not errors, errors
        browser.close()
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    assert con.execute(f"SELECT kind FROM features.traffic WHERE osm_id = {j[0]}").fetchall() == [("traffic_signals",)], "the new signal is in features.traffic"
    assert {r[0] for r in con.execute(f"SELECT lanes FROM driving.edges WHERE osm_id = {ROAD}").fetchall()} == {7}
    assert {r[0] for r in con.execute(f"SELECT walk_type FROM walking.edges WHERE osm_id = {ROAD}").fetchall()} == {"sidewalk"}, "the sidewalk tag changes the walking type"
    assert con.execute(f"SELECT map_extract(tags, 'building:levels')[1], map_extract(tags, 'roof:shape')[1], map_extract(tags, 'roof:colour')[1] FROM features.buildings "
                       f"WHERE osm_id = {bid} AND osm_type = 'way'").fetchall() == [("7", "gabled", "#cc3300")]
    assert con.execute(f"SELECT map_extract(tags, 'traffic_signals:direction')[1], map_extract(tags, 'note:test')[1] FROM features.traffic WHERE osm_id = {nid}").fetchall() == [("backward", "hello")]
    con.close()
