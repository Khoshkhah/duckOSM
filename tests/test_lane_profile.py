"""The lane profile's rules (docs/design/gmns_lane_profile.md), one tag case per row: tags in, lanes out, left to right."""
import pytest

from duckosm.processors.lane_profile import lanes_of

# (case, tags, keywords, [(lane_num, use, source)])
CASES = [
    ("untagged one-way: one default lane", {"highway": "primary", "oneway": "yes"}, dict(oneway=True, n_motor=1),
     [(1, "auto", "default")]),
    ("lanes=2 one-way", {"highway": "primary", "oneway": "yes", "lanes": "2"}, dict(oneway=True, n_motor=2, n_source="lanes"),
     [(1, "auto", "lanes"), (2, "auto", "lanes")]),
    ("more turn:lanes entries than lanes: one lane per entry", {"lanes": "2", "turn:lanes": "left|through|right"}, dict(n_motor=2, n_source="lanes"),
     [(1, "auto", "turn:lanes"), (2, "auto", "turn:lanes"), (3, "auto", "turn:lanes")]),
    ("bus:lanes marks its lane", {"lanes": "3", "bus:lanes": "||designated"}, dict(oneway=True, n_motor=3, n_source="lanes"),
     [(1, "auto", "lanes"), (2, "auto", "lanes"), (3, "bus", "bus:lanes")]),
    ("lanes:bus=1: the right-most motor lane", {"lanes": "3", "lanes:bus": "1"}, dict(oneway=True, n_motor=3, n_source="lanes"),
     [(1, "auto", "lanes"), (2, "auto", "lanes"), (3, "bus", "lanes:bus")]),
    ("busway:right=lane on the forward edge", {"lanes": "2", "busway:right": "lane"}, dict(oneway=True, n_motor=2, n_source="lanes"),
     [(1, "auto", "lanes"), (2, "bus", "busway:right")]),
    ("busway:right=lane is not the reverse edge's", {"lanes": "2", "busway:right": "lane"}, dict(reverse=True, n_motor=1, n_source="lanes"),
     [(1, "auto", "lanes")]),
    ("cycleway:right=lane: a bike lane right of the motor lanes, not counted in lanes", {"lanes": "2", "oneway": "yes", "cycleway:right": "lane"},
     dict(oneway=True, n_motor=2, n_source="lanes"), [(1, "auto", "lanes"), (2, "auto", "lanes"), (3, "bike", "cycleway:right")]),
    ("cycleway:left=lane on a one-way, with the traffic: lane -1", {"lanes": "1", "oneway": "yes", "cycleway:left": "lane"},
     dict(oneway=True, n_motor=1, n_source="override"), [(-1, "bike", "cycleway:left"), (1, "auto", "override")]),
    ("cycleway:left=lane against the traffic (oneway:bicycle=no): not on this edge", {"oneway": "yes", "oneway:bicycle": "no", "cycleway:left": "lane"},
     dict(oneway=True, n_motor=1), [(1, "auto", "default")]),
    ("on a two-way road cycleway:left is the reverse edge's right", {"lanes": "2", "cycleway:left": "lane"}, dict(reverse=True, n_motor=1, n_source="lanes"),
     [(1, "auto", "lanes"), (2, "bike", "cycleway:left")]),
    ("a bus-only edge (contraflow bus lane), bikes sharing it", {"oneway": "yes", "oneway:bus": "no", "cycleway:left": "share_busway"},
     dict(reverse=True, bus_only=True, shared_bike=True), [(1, "bus,bike", "access=bus")]),
    ("share_busway on a bus lane of this side", {"lanes": "2", "bus:lanes": "|designated", "cycleway:right": "share_busway"},
     dict(oneway=True, n_motor=2, n_source="lanes"), [(1, "auto", "lanes"), (2, "bus,bike", "bus:lanes + cycleway share_busway")]),
]


@pytest.mark.parametrize("case,tags,kw,want", CASES, ids=[c[0] for c in CASES])
def test_a_tag_case(case, tags, kw, want):
    got = lanes_of(tags, **kw)
    assert [(x["lane_num"], x["use"], x["source"]) for x in got] == want


def test_widths_say_where_they_come_from():
    got = lanes_of({"lanes": "2", "width:lanes": "3.5|", "cycleway:right": "lane", "cycleway:right:width": "1.8"}, highway="residential",
                   oneway=True, n_motor=2, n_source="lanes")
    assert [(x["width_m"], x["width_source"]) for x in got] == [(3.5, "width:lanes"), (3.0, "default"), (1.8, "cycleway:right:width")]
    assert lanes_of({}, highway="service")[0]["width_m"] == 2.5 and lanes_of({}, highway="service")[0]["width_source"] == "default"
