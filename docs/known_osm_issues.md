# Known OSM data issues

A running catalogue of **OpenStreetMap source-data problems** that surface in the duckOSM output.

These are cases where duckOSM is behaving **correctly given its input** — the data is wrong (or
ambiguous) in OSM itself — so the fix usually belongs **upstream in OSM** (re-tag the element and
rebuild), or, when that is impractical, as a **local override** in the duckOSM pipeline. Pure duckOSM
logic bugs do **not** belong here; fix those in code + tests.

For each entry record: the affected element(s), the symptom in the output, the verified root cause,
the recommended fix, and a status.

---

## 1. Hökens Gata — missing `oneway` tag (rendered two-way)

| field | value |
| --- | --- |
| **Area** | Södermalm (`pbf/sodermalm_pbf.osm.pbf`) |
| **OSM way** | `4392632` (`highway=residential`, `name=Hökens Gata`) |
| **Edges** | `6739996069361458541` (forward) + `491685589105352415` (reverse twin) |
| **Status** | **CONFIRMED OSM error** — the adjacent Hökens Gata segment is tagged `oneway=yes`; way `4392632` is just missing the tag |

**Symptom.** Hökens Gata comes out as a **two-way** street: one OSM way → a forward edge plus a
reverse twin (same `osm_id`, identical length, mirror-image geometry, `oneway=False`). A user expected
it to be one-way.

**Confirmed (it IS one-way).** The *adjacent* Hökens Gata segment — edge `8115774911033883256`, OSM way
`676781760`, which connects end-to-end with `4392632` — **is** explicitly tagged `oneway=yes`
(`highway=residential, name=Hökens Gata, oneway=yes, oneway:bicycle=no, maxspeed=30, …`). So the street
is genuinely one-way; way `4392632` is simply **missing the tag**, which is why only that stretch came
out two-way. This is a real OSM tagging error, not a duckOSM bug.

**Root cause (verified).** The raw OSM way `4392632` carries **no `oneway` tag** (and no
`junction=roundabout`). Its full tag set is:

```
highway=residential, name=Hökens Gata, maxspeed=30, lit=yes,
sidewalk=both, sidewalk:both:surface=paving_stones, surface=sett
```

Per the OSM convention an untagged ordinary road is **two-way by default**, which is exactly what
`RoadFilter._oneway_expression` (`src/duckosm/processors/road_filter.py`) encodes — the `ELSE FALSE`
branch — and `GraphBuilder` then gives every `NOT oneway` edge a reverse twin. So **the output is
correct for the input**; if the street is one-way on the ground, the gap is the **missing OSM tag**,
not a duckOSM bug. (Confirmed by reading the way's tags straight from the pbf with `ST_READOSM`.)

**Recommended fix.**
1. Verify the real direction of travel on the ground / from imagery.
2. If it is genuinely one-way: add `oneway=yes` (or `-1`) to way `4392632` in **OpenStreetMap**, then
   rebuild the area. This benefits every downstream OSM consumer, not just duckOSM.
3. If you cannot wait on OSM: add a **local override** in the pipeline (force `oneway=TRUE` for this
   `osm_id`) before `GraphBuilder` adds reverse edges.

**Note — related duckOSM hardening (separate from this data error).** While investigating, the
implicit-one-way rule for `motorway` / `motorway_link` was added to `_oneway_expression` (untagged
motorways are implicitly one-way per OSM convention, with an explicit `oneway=no` still winning). That
is a logic improvement and is a **no-op on Södermalm** (0 motorways) — it does not change the Hökens
Gata result, which stays two-way because residential roads are correctly two-way-by-default.

---

## 2. Katarina Bangata — missing `oneway` tag (rendered two-way)

| field | value |
| --- | --- |
| **Area** | Södermalm (`pbf/sodermalm_pbf.osm.pbf`) |
| **OSM way** | `233761079` (`highway=residential`, `name=Katarina Bangata`) |
| **Edges** | `224569842445438623` (forward) + `8329252562982974380` (reverse twin) |
| **Status** | OPEN — pending on-the-ground verification of the real directionality |

Same class as #1. The raw way `233761079` carries **no `oneway` tag** and no `junction=roundabout`
(full tags: `highway=residential`, `lit=yes`, `maxspeed=30`, `name=Katarina Bangata`,
`parking:both=no`, `parking:both:restriction=no_parking`, `surface=asphalt`, `wikidata=Q1692894`,
`wikipedia=sv:Katarina Bangata`). duckOSM therefore correctly defaults it to **two-way** (forward +
reverse twin) — the output is faithful to the input. If it is one-way on the ground, add
`oneway=yes` / `-1` to way `233761079` in OpenStreetMap and rebuild (or apply a local override).
Verified by reading the tags from the pbf with `ST_READOSM`.

---

## 3. Trans-Canada Highway westbound (Second Narrows approach) — undercounted `lanes`

| field | value |
| --- | --- |
| **Area** | Vancouver (City of Vancouver, built from `metro_vancouver.osm.pbf` via `config/vancouver_city.yaml`) |
| **OSM way** | `507979055` (`highway=motorway`, `name=Trans-Canada Highway`, one-way westbound) |
| **Edge** | `1868603694539326257` (single directed edge — the way is one-way) |
| **Status** | OPEN — probable OSM under-count; BC MoTI per-lane counts show **3** WB lanes vs OSM `lanes=2` |

**Symptom.** The westbound Trans-Canada carriageway near the east end of the Second Narrows crossing
comes out with `lanes=2`, but three separate BC MoTI **per-lane** traffic-count stations sit across it,
implying **3** through lanes.

**Evidence (BC MoTI).** The `aadt_moti` map-match (in the `vancouver_traffic_data` project) landed
three per-lane WB stations on this one directed edge:

- `P-15-21` "WB SLOW LANE" — AADT 13,115
- `P-15-22` "WB MIDDLE LANE" — AADT 23,404
- `P-15-23` "WB FAST LANE" — AADT 22,566  → **3 lanes, ≈59,085 veh/day total**

The **eastbound twin** (OSM way `23795653`) *is* correctly tagged `lanes=3` (with full
`turn:lanes` / `destination:lanes`), so the WB carriageway reading `2` is an asymmetry that points to a
tagging gap, not a real lane drop.

**Root cause (verified).** Raw OSM way `507979055` is tagged `lanes=2`, `oneway=yes`, with no
`lanes:forward`/`lanes:backward`. duckOSM's one-way branch in
`RoadFilter._create_ways_table` (`src/duckosm/processors/road_filter.py`) takes
`COALESCE(n_total, class_default)` = the tagged **2**, so the output is faithful to the input — the
count is wrong in **OSM**, not in duckOSM. (Confirmed by reading the way's tags from the pbf.)

**Caveat.** The MoTI reading is old (`LAST_YEAR = 2002`); a lane count is structural and unlikely to
have dropped since, but confirm against current imagery before editing OSM — and check the exact
station location isn't a spot where a temporary merge/auxiliary lane briefly makes 3.

**Recommended fix.**
1. Verify on current imagery / on the ground that the WB carriageway carries 3 through lanes here.
2. If so, set `lanes=3` on way `507979055` in **OpenStreetMap** and rebuild the area.
3. If you cannot wait on OSM, apply a **local override** for this `osm_id` in the pipeline.

> **Not listed here:** Lions Gate Bridge / Stanley Park Causeway reading `lanes=1` per direction was a
> **duckOSM logic bug** (ignored `lanes:reversible`), fixed in code — OSM tags them correctly
> (`lanes=3` + `lanes:reversible=1`). Per this file's scope, code bugs are fixed in code + tests, not
> catalogued here.
