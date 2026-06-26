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
| **Status** | OPEN — pending on-the-ground verification of the real directionality |

**Symptom.** Hökens Gata comes out as a **two-way** street: one OSM way → a forward edge plus a
reverse twin (same `osm_id`, identical length, mirror-image geometry, `oneway=False`). A user expected
it to be one-way.

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
