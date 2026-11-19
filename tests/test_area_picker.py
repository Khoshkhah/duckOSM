"""duckosm.AreaPicker (src/duckosm/widget.py): the widget's area to the arguments of duckosm.get(), with no browser and
no network."""

import json

import pytest

pytest.importorskip("anywidget")
from duckosm import widget  # noqa: E402


@pytest.mark.parametrize("area, kw", [
    ({"mode": "osm_id", "osm_id": "2220322", "name": "Monaco"}, {"osm_id": 2220322, "name": "Monaco"}),
    ({"mode": "place", "place": "Monaco"}, {"place": "Monaco"}),
    ({"mode": "bbox", "bbox": [7.40, 43.72, 7.44, 43.76]}, {"bbox": (7.40, 43.72, 7.44, 43.76)}),
    ({"mode": "h3", "h3_cell": "883969a40bfffff"}, {"h3_cell": "883969a40bfffff"}),
])
def test_get_args(area, kw):
    assert widget.get_args(area) == kw


@pytest.mark.parametrize("area", [{}, {"mode": "bbox", "bbox": [7.44, 43.72, 7.40, 43.76]}])
def test_no_area_is_an_error(area):
    with pytest.raises(ValueError):
        widget.get_args(area)


def test_boundary_goes_to_a_file(monkeypatch):
    geojson = {"type": "Polygon", "coordinates": [[[7.4, 43.72], [7.44, 43.72], [7.44, 43.76], [7.4, 43.72]]]}
    calls = []
    fake = lambda **kw: calls.append(kw) or {"name": "x", "extract": {}}  # noqa: E731
    monkeypatch.setattr("duckosm._api.get", fake)
    monkeypatch.setattr("duckosm.get", fake, raising=False)
    p = widget.AreaPicker(cache="c")
    p.area = {"mode": "boundary", "geojson": geojson}
    kw = p._kwargs()
    with open(kw["boundary"]) as f:
        assert json.load(f) == geojson
    assert kw["name"] == "area" and kw["cache"] == "c"


def test_get_waits_for_the_get_button(monkeypatch):
    import time
    calls = []

    def fake(**kw):
        calls.append(kw)
        if kw.get("dry_run"):
            return {"name": "x", "extract": {}}
        time.sleep(0.3)
        return "the area"
    monkeypatch.setattr("duckosm._api.get", fake)
    monkeypatch.setattr("duckosm.get", fake, raising=False)
    p = widget.AreaPicker()
    p.area = {"mode": "h3", "h3_cell": "883969a40bfffff"}
    p._on_msg(None, {"type": "get"}, [])       # the button
    assert p.get() == "the area" == p.result   # waits for the button's run
    assert sum(not kw.get("dry_run") for kw in calls) == 1
