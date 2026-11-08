"""The Road and Node tabs of the fixes editor in a browser (headless Chromium, Playwright). These checks were first written for Studio's end-to-end test of the same
editor; they live here so that the editor is tested where it is made. The module has its own Monaco build and page server (editor_fixtures.py)."""
import pytest

from editor_fixtures import ROAD, _QUERY, _open_clean, _pick_road, monaco, page_url, pw  # noqa: F401

_PICKED = "(window.fixesPanel.drawn().sel + (window.fixesPanel.drawn().lit['fix-sel'] ?? 0))"       # the picked object on the map: the editor's layer or the highlight set
_AMBER = "(window.fixesPanel.drawn().edits + (window.fixesPanel.drawn().lit['fix-edits'] ?? 0))"
_POPUPS = "document.querySelectorAll('.maplibregl-popup:not(.rs-tip)').length"


def _rows(page, sql):
    return page.evaluate(_QUERY, sql)


def _at(page, lon, lat):
    """Move the map to the point (zoom 18) and give the pixel of the point on the page, when the map stands still."""
    page.evaluate(f"window.map.jumpTo({{center: [{lon}, {lat}], zoom: 18}})")
    page.wait_for_function("window.map.loaded() && !window.map.isMoving()", timeout=30_000)
    return page.evaluate(f"(() => {{ const p = map.project([{lon}, {lat}]), c = map.getCanvas().getBoundingClientRect(); return [p.x + c.left, p.y + c.top]; }})()")


def _signal(page):
    """A traffic signal of the sample: its OSM id and place."""
    return _rows(page, "SELECT osm_id AS id, ST_X(geom) AS lon, ST_Y(geom) AS lat FROM features.traffic WHERE kind LIKE '%signal%' ORDER BY osm_id LIMIT 1")[0]


def _load_road(page, osm_id):
    """The Load button of the Road tab (the id is typed, as a person does)."""
    page.fill("#fix-id", str(osm_id))
    page.click("#fix-load")
    page.wait_for_function(f"document.getElementById('fix-obj').textContent.includes('way {osm_id}') && document.querySelector('#fixf-lanes')", timeout=30_000)


def test_a_click_on_a_road_fills_the_form_and_opens_no_popup(monaco, page_url):
    """Replaces Studio line 130. A picked road shows its way id and tag count, the form starts with the data of the road (lanes, one-way), and no popup opens."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        _pick_road(page, ROAD)
        page.wait_for_function("document.querySelector('#fixf-lanes') && document.querySelector('#fix-form select')", timeout=30_000)
        assert page.evaluate(_POPUPS) == 0, "no popup in the fixes panel"
        assert f"way {ROAD}" in page.inner_text("#fix-obj") and "Road" in page.inner_text("#fix-obj")
        n_tags = int(page.inner_text("#fix-obj .alltags summary").split("(")[1].split(")")[0])
        assert n_tags >= 2, "all the tags of the road are listed"
        edges = _rows(page, f"SELECT any_value(lanes)::INTEGER AS lanes, bool_or(oneway) AS oneway FROM driving.edges WHERE osm_id = {ROAD}")[0]
        if edges["lanes"] is not None:
            assert page.input_value("#fixf-lanes") == str(edges["lanes"]), "the form starts with the lanes of the data"
        assert page.input_value("#fixf-oneway") == str(bool(edges["oneway"])).lower(), "the one-way field has the data"
        assert not errors, errors
        browser.close()


def test_a_road_rule_with_preset_fields_and_guards(monaco, page_url):
    """Replaces Studio line 157. Lanes go into the rule, sidewalk and surface into extra.tag_edits, the road is in the amber layer, an edited label has the mark, Reset form shows the
    saved rule again, and a guard closes the fields that cannot be set together (one-way closes the lane counts of each direction, a bridge closes the tunnel list)."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        page.evaluate("fixesPanel.tab('road')")
        _load_road(page, ROAD)
        assert page.input_value("#fixf-sidewalk") == "", "no sidewalk tag in the data"
        # the lists are roadstyle's select boxes: the native <select> is hidden, so the test sets it with force
        page.select_option("#fixf-sidewalk", "both", force=True)
        page.select_option("#fixf-surface", "paved", force=True)
        page.fill("#fixf-lanes", "7")
        page.select_option("#fixf-oneway", "true", force=True)
        assert page.is_disabled("#fixf-lanes_forward") and page.is_disabled("#fixf-lanes_backward") and "A one-way road" in page.inner_text("#fix-form")
        assert page.input_value("#fixf-lanes") == "7", "the guard keeps the other fields"
        page.select_option("#fixf-oneway", "false", force=True)
        assert not page.is_disabled("#fixf-lanes_forward"), "a two-way road opens the lane counts again"
        page.select_option("#fixf-bridge", "yes", force=True)
        assert page.is_disabled("#fixf-tunnel") and "bridge or a tunnel" in page.inner_text("#fix-form")
        page.select_option("#fixf-bridge", "", force=True)
        assert not page.is_disabled("#fixf-tunnel")
        page.click("#fix-save")
        page.wait_for_function("fixesPanel.count() === 1", timeout=10_000)
        y = page.evaluate("window.fixesPanel.text() ?? ''")
        assert f"osm_id: {ROAD}" in y and "lanes: 7" in y, y
        assert "tag_edits" in y and "sidewalk: both" in y and "surface: paved" in y, y
        page.wait_for_function(f"{_AMBER} > 0", timeout=20_000)
        assert "edited" in (page.get_attribute("label[for=fixf-sidewalk]", "class") or "")
        page.fill("#fixf-lanes", "9")
        page.click("#fix-revert")
        assert page.input_value("#fixf-lanes") == "7", "Reset form: the form shows the saved rule again"
        assert not errors, errors
        browser.close()


def test_the_tabs_keep_their_own_objects(monaco, page_url):
    """Replaces Studio line 180. The Building tab has its own empty form (and clears roadstyle's selection), the Road tab keeps its road and its rule, and the map shows the road of
    the tab again (blue, with its rule amber)."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        _pick_road(page, ROAD)
        page.fill("#fixf-lanes", "7")
        page.click("#fix-save")
        page.wait_for_function("fixesPanel.count() === 1", timeout=10_000)
        page.evaluate("window.__desel = 0; const o = window.rsDeselect; window.rsDeselect = (...a) => { window.__desel++; return o?.(...a); }")
        page.click("#fix-tab-building")
        assert page.evaluate("window.__desel") >= 1, "a switch of tab clears roadstyle's selection on the map"
        assert page.locator("#fixf-lanes").count() == 0 and "Nothing is picked" in page.inner_text("#fix-obj"), page.inner_text("#fix-obj")
        assert page.input_value("#fix-id") == ""
        page.wait_for_function(f"{_PICKED} === 0", timeout=10_000)                      # the Building tab has nothing picked, so nothing is blue
        page.click("#fix-tab-road")
        page.wait_for_function(f"document.getElementById('fix-id').value === '{ROAD}' && document.querySelector('#fixf-lanes')?.value === '7'", timeout=20_000)
        page.wait_for_function(f"{_PICKED} > 0 && (window.fixesPanel.drawn().lit['fix-edits'] ?? 0) > 0", timeout=20_000)
        # the map shows the road of the tab
        pt = _rows(page, f"SELECT ST_X(ST_PointOnSurface(ST_Union_Agg(geometry))) AS x, ST_Y(ST_PointOnSurface(ST_Union_Agg(geometry))) AS y FROM driving.edges WHERE osm_id = {ROAD}")[0]
        page.wait_for_function(f"!window.map.isMoving() && window.map.getBounds().contains([{pt['x']}, {pt['y']}])", timeout=20_000)
        assert not errors, errors
        browser.close()


def test_unselect_takes_the_road_out_and_load_brings_it_back(monaco, page_url):
    """Replaces Studio line 188. Unselect empties the form, the id and the blue road of the tab (the rule stays), and Load shows the road with its rule again."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        _pick_road(page, ROAD)
        page.fill("#fixf-lanes", "7")
        page.click("#fix-save")
        page.wait_for_function("fixesPanel.count() === 1", timeout=10_000)
        page.wait_for_function(f"{_PICKED} > 0", timeout=20_000)
        assert page.is_visible("#fix-unselect")
        page.click("#fix-unselect")
        assert page.locator("#fixf-lanes").count() == 0 and page.input_value("#fix-id") == "" and not page.is_visible("#fix-unselect")
        page.wait_for_function(f"{_PICKED} === 0", timeout=10_000)
        assert page.evaluate("window.fixesPanel.count()") == 1, "Unselect does not touch the rule"
        _load_road(page, ROAD)
        # the road has a rule: the box asks first, and Edit it opens the form with the value of the rule
        page.click("#fix-inchg [data-ask=edit]")
        page.wait_for_function("document.querySelector('#fixf-lanes')?.value === '7'", timeout=10_000)
        page.wait_for_function(f"{_PICKED} > 0", timeout=20_000)
        assert not errors, errors
        browser.close()


def test_the_node_tab_draws_the_dots_and_the_picked_node_has_a_ring(monaco, page_url):
    """Replaces Studio line 359. The Node tab draws the traffic signals and crossings in view as dots (with a legend and the count), a hover gives a ring, a pointer cursor and a
    tooltip with the kind and the id, and a click picks the node: its form opens, and it has the blue ring."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        n = _signal(page)
        page.click("#fix-tab-node")
        px = _at(page, n["lon"], n["lat"])
        assert page.is_visible("#fix-nodes-legend") and "Traffic signal" in page.inner_text("#fix-nodes-legend")
        page.wait_for_function("window.fixesPanel.drawn().nodes > 0 && map.queryRenderedFeatures({layers: ['fix-nodes-pt']}).length > 0", timeout=20_000)
        page.wait_for_function("document.getElementById('fix-nodes-info').textContent.includes('In view:')", timeout=10_000)
        assert "signal" in page.inner_text("#fix-nodes-info")
        page.mouse.move(*px)
        page.wait_for_selector(".maplibregl-popup.fix-tip", timeout=10_000)
        tip = page.inner_text(".maplibregl-popup.fix-tip")
        assert "Traffic signal" in tip and f"node {n['id']}" in tip, tip
        assert page.evaluate("map.getCanvas().style.cursor") == "pointer"
        page.wait_for_function("map.queryRenderedFeatures({layers: ['fix-nodes-hover']}).length > 0", timeout=10_000)
        page.mouse.move(px[0] + 300, px[1] + 200)                                      # away from the dot: no tooltip
        page.wait_for_function("document.querySelectorAll('.maplibregl-popup.fix-tip').length === 0", timeout=10_000)
        page.mouse.click(*px)
        page.wait_for_function(f"document.getElementById('fix-id').value === '{n['id']}' && document.querySelector('#fixf-traffic_signals_direction')", timeout=20_000)
        page.wait_for_function("map.queryRenderedFeatures({layers: ['fix-sel-pt']}).length > 0", timeout=20_000)
        assert page.evaluate(_POPUPS) == 0, "no popup in the fixes panel"
        assert not errors, errors
        browser.close()


def test_a_node_preset_field_an_added_tag_and_a_deleted_tag(monaco, page_url):
    """Replaces Studio line 375. A node of the Node tab: a preset field (the direction of the signal) goes into extra.tag_edits, a tag that the preset does not list is added with
    Add, and a data tag is deleted with its x button (it goes into `delete`)."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        n = _signal(page)
        page.evaluate("fixesPanel.tab('node')")
        page.evaluate(f"fixesPanel.select('node', 'node', {n['id']})")
        page.wait_for_function(f"document.getElementById('fix-id').value === '{n['id']}' && document.querySelector('#fixf-traffic_signals_direction')", timeout=30_000)
        text = lambda: page.evaluate("window.fixesPanel.text() ?? ''")
        page.select_option("#fixf-traffic_signals_direction", "backward", force=True)
        page.click("#fix-save")
        page.wait_for_function("fixesPanel.count() === 1", timeout=10_000)
        y = text()
        assert "type: node" in y and f"id: {n['id']}" in y and "traffic_signals:direction: backward" in y, y
        # a tag that the preset does not list: add it
        page.wait_for_function("document.querySelector('#fix-addkey')", timeout=10_000)
        page.fill("#fix-addkey", "note:test")
        page.fill("#fix-addval", "hello")
        page.click("#fix-add")
        page.click("#fix-save")
        assert "note:test: hello" in text(), text()
        # delete a data tag
        page.wait_for_selector("#fix-form [data-clear='traffic_signals']", timeout=10_000)
        page.click("#fix-form [data-clear='traffic_signals']")
        page.click("#fix-save")
        y = text()
        assert "delete:" in y and "- traffic_signals" in y, y
        assert page.evaluate("window.fixesPanel.count()") == 1, "the three changes are one record of the node"
        assert not errors, errors
        browser.close()


def test_a_node_with_two_roads_takes_no_traffic_signal(monaco, page_url):
    """Replaces Studio line 652. The constraints of the Node tab: a plain road node (two roads meet) cannot be a traffic signal. The option Traffic signals of the Kind field is
    off, and the reason ("at least 3 roads") is under the field."""
    with pw.sync_playwright() as p:
        browser = p.chromium.launch()
        page, errors = _open_clean(browser, page_url)
        plain = _rows(page, "SELECT nid::BIGINT AS id FROM (SELECT source AS nid, target AS o FROM driving.edges UNION ALL SELECT target, source FROM driving.edges) GROUP BY nid "
                            "HAVING count(DISTINCT o) = 2 AND nid IN (SELECT node_id FROM driving.way_nodes) AND nid NOT IN (SELECT osm_id FROM features.traffic) ORDER BY nid LIMIT 1")[0]["id"]
        page.evaluate("fixesPanel.tab('node')")
        page.evaluate(f"fixesPanel.select('node', 'node', {plain})")
        page.wait_for_function(f"document.getElementById('fix-obj').textContent.toLowerCase().includes('node {plain}') && document.querySelector('#fixf-highway')", timeout=30_000)
        assert page.evaluate("document.querySelector('#fixf-highway option[value=traffic_signals]').disabled")
        assert "at least 3 roads" in page.inner_text("#fix-form .quickwhy"), page.inner_text("#fix-form")
        page.click("#fix-unselect")
        assert not errors, errors
        browser.close()
