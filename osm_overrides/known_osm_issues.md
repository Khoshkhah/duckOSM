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
| **Status** | **WITHDRAWN 2026-09-10** — the override was wrong and is removed. This stretch is **two-way**; OSM is right to omit the tag. |

Same class as #1. The raw way `233761079` carries **no `oneway` tag** and no `junction=roundabout`
(full tags: `highway=residential`, `lit=yes`, `maxspeed=30`, `name=Katarina Bangata`,
`parking:both=no`, `parking:both:restriction=no_parking`, `surface=asphalt`, `wikidata=Q1692894`,
`wikipedia=sv:Katarina Bangata`), so duckOSM correctly defaults it to **two-way** — faithful to the input.

**Withdrawn — the street is NOT uniformly one-way.** The 2026-07 Overpass count of "56 segments
tagged `oneway=yes`" was taken across every way named *Katarina Bangata*, and read as proof that the
untagged ones were omissions. Checked against two sources on 2026-09-10:

| source | way `233761079` |
| --- | --- |
| OSM raw tags (this extract) | no `oneway` — two-way |
| Overture Maps (`w233761079@6`) | `access_restrictions` NULL — two-way |

And Overture's 22 Katarina Bangata segments split **11 two-way / 11 one-way**, so the street changes
character along its length rather than being one-way with a few tags missing. Kaveh confirmed on the
ground that this stretch is two-way. The left turn at node `1392932069` is **allowed** on the ground - the
junction there is laid out differently from what the edge geometry suggests - so no turn-restriction
override belongs here. duckOSM permitting that movement was never the defect; the suppressed reverse
edge was.

**What the wrong override cost.** `oneway: true` is *topological* — it suppresses the reverse twin.
With the reverse edge gone, node `1392932069` had 8 veh/h arriving and 360 leaving in the Södermalm
flow-map priors: a 45x conservation break that looked like a matching error and was not one. The
lesson is in the file's own warning, which was already there: *"an unverified `oneway` flip silently
drops the reverse edge."* Here it was flipped on evidence that had not been checked against the
individual way.

**A related gap.** `oneway` in the rules file is a **bool**, so it can only mean "one-way along the
way's digitisation". OSM itself distinguishes `oneway=yes` from `oneway=-1`, and `road_filter.py`
already reads both. A rule cannot currently express "one-way against the node order", so any street
that needs that correction cannot be fixed here at all.

**Fix.** Add `oneway=yes` to way `233761079` in OpenStreetMap (benefits every consumer), or — as now
applied — a local override (`osm_id: 233761079, oneway: true`) in `osm_overrides.yaml`.

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

---

## 4. Bohusgatan — missing `oneway` tag on 5 of its 6 ways (rendered two-way)

| field | value |
| --- | --- |
| **Area** | Södermalm (`pbf/sodermalm_pbf.osm.pbf`) |
| **OSM ways** | untagged: `140726414` (asked), `34425155`, `34425156`, `1277761939`, `1277761941` · **tagged `oneway=yes`: `151083837`** |
| **Edges (way 140726414)** | `3351157701764431365` (forward) + `7237647270561809940` (reverse twin) |
| **Status** | **CONFIRMED OSM error** — a Bohusgatan segment (way `151083837`) is `oneway=yes`; the other five ways are missing the tag. (Verify the *whole* street is one-way, not just that stretch.) |

Same class as #1/#2, at larger scale: **five of Bohusgatan's six ways carry no `oneway` tag** (e.g. way
`140726414`: `highway=residential`, `name=Bohusgatan`, `maxspeed=30`, `lit=yes`, `parking:both=no`,
`surface=asphalt`), so duckOSM renders them two-way. But one segment — **way `151083837` — is tagged
`oneway=yes`**, strong evidence Bohusgatan is a one-way street whose other segments simply lack the tag.
Fix: add `oneway=yes` (matching way `151083837`'s direction) to the five untagged ways in OSM and
rebuild, or apply a local override. Verified via `ST_READOSM`.

---

## 5. Tartu underground-parking ramp — whole way mis-tagged `layer=-1`

| field | value |
| --- | --- |
| **Area** | Tartu (`config/tartu.yaml`) |
| **OSM way** | `1429337399` (`highway=service`, `service=driveway`, `layer=-1`) — an underground-parking ramp |
| **Edges** | split at junction node `13139650528` into `761090243668882770` (17.4 m, junction side) + `2063175062303482770` (75.2 m) — plus reverse twins; **all inherit `layer=-1`** |
| **Neighbour** | way `1429337400` (`6219894338208087916`, `service=driveway`, no `layer` = grade) **joins at the shared node** `13139650528` |
| **Status** | **CONFIRMED OSM mapping shortcut** — override **enabled** in `osm_overrides.yaml`. |

The ramp descends from the surface (where it meets the level-0 driveway `1429337400` at a shared node)
down to underground parking, but the **whole OSM way carries a single `layer=-1`** — there is no
`tunnel`/`bridge` structure (`layer` is just a relative stacking number, not a physical object). When
duckOSM splits the way at the junction, **every segment inherits `layer=-1`**, so the surface end
(`761090…`) is stacked below — and, because the renderer styles anything `layer<0` like a tunnel, drawn
faded/dashed — even though it is at grade with the driveway it joins.

Properly this way should be **split in OSM at the level-transition node** into a `layer=0` part and a
`layer=-1` part. As a shortcut we bring the whole way to grade: local override
`osm_id: 1429337399, layer: 0` in `osm_overrides.yaml` (there is no real tunnel, so rendering it
at grade end-to-end is acceptable). Takes effect on the next rebuild.

---

## 6. Tartu Tähe junction — missing `no_u_turn` (spurious U-turn onto opposing carriageway)

| field | value |
| --- | --- |
| **Area** | Tartu (`config/tartu.yaml`) |
| **Junction** | node `330045016` (two-way Tähe meets a one-way link) |
| **OSM ways** | from `1307524008` (one-way inbound) · via node `330045016` · to `997402723` (Tähe, opposing/westbound) |
| **Movement** | `2048307190992484263 → 5604449516676458023` (typed `uturn`, mvmt_code `EBU`) |
| **Status** | **CONFIRMED OSM gap** — no restriction relation at the node; synthetic override **enabled** (`turn_restrictions:` in `osm_overrides.yaml`; takes effect on the next driving rebuild) |

At node `330045016` the one-way link `2048307190992484263` (way `1307524008`) arrives heading ~101°
(ESE). It has a legitimate `thru` onto **eastbound** Tähe (`8788328652948037784`, way `207110404`,
leaves ~124°). But duckOSM also emits a movement onto the **westbound / opposing** Tähe carriageway
(`5604449516676458023`, way `997402723`, leaves ~295°) — a **−166° turn back onto oncoming traffic**,
typed `uturn`. That maneuver is physically forbidden at this merge; OSM simply carries **no
`restriction=no_u_turn` relation** there, so duckOSM — which connects all node-adjacent edge pairs
minus same-way immediate reversals — generates it. (This is why it surfaced in the lanestyle
lane-connectivity view as an extra "outgoing" from that lane.)

**Fix.** Add the restriction upstream in OSM (`type=restriction`, `restriction=no_u_turn`; from way
`1307524008`, via node `330045016`, to way `997402723`) and rebuild — or, locally, the synthetic
turn-restriction override in `osm_overrides.yaml` (mechanism in
`docs/design/turn-restriction-overrides.md`). Verified against the movement / edge-graph tables: the
`thru` movement is correct and untouched; only the U-turn is spurious.

## 7. Katarina Västra Kyrkogata — missing `oneway` tag on way 120860763 (rendered two-way)

| field | value |
| --- | --- |
| **Area** | Södermalm (`config/sodermalm.yaml`) |
| **OSM way** | `120860763` (`highway=residential`, `name=Katarina Västra Kyrkogata`) |
| **Edges** | `8889359159650646290` (forward, `1354121015 → 194903`) + `6435483818528014804` (reverse twin, `194903 → 1354121015`) |
| **Status** | **CONFIRMED OSM error** — override **enabled** (`osm_overrides.yaml`); takes effect on the next Södermalm rebuild |

Found 2026-09-27 from SonoFlow's Model Explorer: the reverse twin `6435483818528014804` had no
turning movement at all - its only way on at node `1354121015` was a U-turn back onto itself.
At that node the street continues as way `1280759567` (same name, **`oneway=yes`**), which runs
`11888148255 → 1354121015` INTO the node, and the traffic goes on along `120860763` forward. So the
street is one-way end to end; `120860763` carries only `highway, name, maxspeed=30, lit, surface`
and no `oneway`, and the OSM default made it two-way. The reverse twin runs against the one-way.

**The other candidates, checked 2026-09-27 against LIVE OpenStreetMap.** In SonoFlow's Södermalm
network, 12 of the 28 edges with no turning movement end at a node where a one-way way begins or
ends. For each, the question is whether a SAME-NAME one-way continues in this way's own drawn
direction - only then does `oneway: true` remove the right edge (it keeps the forward one):

| way | street | verdict |
| --- | --- | --- |
| `120860763` | Katarina Västra Kyrkogata | **enabled** - continues `1280759567` forward |
| `1280522019` | Kapellgränd | **enabled** - continues `323129897` forward |
| `140726414` | Bohusgatan | **enabled** - continues `151083837` forward (also issue #4) |
| `1280486052` | Hökens Gata | not needed - live OSM now tags it `oneway=yes`; a newer extract fixes it |
| `1197353428` | Maria Prästgårdsgata | one-way AGAINST its drawn direction (continues `30678670` in reverse). `oneway: true` would drop the wrong edge; needs a reversed override, which this file cannot express yet |
| `525976008` | (service) | unsure - the one-way it meets (`106080038`) is also unnamed, so "same name" proves nothing |
| `24488730`, `1281907887`, `388628833`, `1280486031`, `24682324`, `821371670` | Timmermansgatan, Tjurbergsgatan, Pustegränd, Peter Myndes Backe, Hornsgatan, Noe Arksfaret | not this error - each ends where a DIFFERENT one-way street arrives, a real dead end for that direction |

**Also found:** `RoadFilter` documents `oneway=-1` as one-way against the drawn direction, but
`GraphBuilder` has no `-1` branch, so such a way would be built in its drawn direction - the wrong
one. Södermalm has none; other areas are unchecked.

## 8. Noe Arksfaret tunnel ramps — tagged drivable, possibly not drivable (UNVERIFIED)

| field | value |
| --- | --- |
| **Area** | Södermalm |
| **OSM ways** | `1422725593` (up, edge `2968865871607388503`) · `1422725594` (down, edge `603796184701625785`) |
| **Status** | **UNVERIFIED** — an impression from Street View (Kaveh, 2026-09-27), not a confirmed fact; rules written but NOT enabled |

Two 21-24 m one-way ramps, `highway=service, tunnel=yes, layer=-1, oneway=yes`, no `access` tag,
joining the underground parking aisle Noe Arksfaret (`821371670`, `service=parking_aisle`,
`indoor=level`) at one end and nothing mapped at the other. OSM's default makes an untagged service
road drivable, so both came into the driving network. On Street View the tunnel part looked as if
it might not be drivable, but that is an opinion, not a confirmation. Found from SonoFlow's Model
Explorer, where the down ramp had no turning movement at all. If it is confirmed, the fix is the
`exclude_modes: [driving]` field (it removes a way from a mode's network before any edge is built,
`OsmOverrides(mode=...)`) - the two rules are written, commented out, in `osm_overrides.yaml` - and
upstream an `access=no` (or `motor_vehicle=no`) tag in OpenStreetMap.

## 9. Monaco roundabout entry - `lanes=2` on a one-lane roundabout

| field | value |
| --- | --- |
| **Area** | Monaco (`data/sample/monaco.osm.pbf`) |
| **OSM way** | `1435532404` (`highway=primary`, `oneway=yes`, `name=Boulevard du Larvotto`, 10 m) |
| **Edge** | `1435532404#1f` |
| **Status** | **CONFIRMED by Street View** (Kaveh, 2026-10-05); override enabled |

**Symptom.** In roadstyle the roundabout bulges where this edge sits: it is drawn twice as wide as
the edges next to it.

**Root cause (verified).** The way carries `lanes=2` in OpenStreetMap itself (version 3, 2026-06-09);
duckOSM copied it unchanged. The adjacent roundabout ways `1435532402` and `1435532403` have
`lanes=1`, and Street View shows a single lane.

**Fix.** `lanes: 1` in `osm_overrides.yaml`; upstream, set `lanes=1` on the way in OpenStreetMap.


## 10. Monaco, Boulevard du Larvotto - `lanes=2` on one-lane one-way ways

| field | value |
| --- | --- |
| **Area** | Monaco (`data/sample/monaco.osm.pbf`) |
| **OSM ways** | `93137596`, `93137569`, `93137578`, `93137560` (Boulevard du Larvotto), `25103774`, `35092477` (Boulevard Louis II), `51691775` (a link), all 11 ways of Avenue Princesse Grace (its `lanes=2` counts the left bike lane: two lanes in all on Street View), `39838824`, `788120420` (Boulevard Princesse Charlotte) (`highway=primary`, `oneway=yes`, `lanes=2`, `name=Boulevard du Larvotto`) |
| **Edges** | `93137596#1f`, `93137569#2f` (and the other pieces of the two ways) |
| **Status** | **CONFIRMED by Street View** (2026-10-10); override enabled |

**Symptom.** The lane map draws two lanes where the street has one.

**Root cause (verified).** The ways carry `lanes=2` in OpenStreetMap; duckOSM reads it as OSM defines it (on a one-way way, every lane runs
its way). Each got `lanes=2` in ONE mass edit, changeset 32866762 (2015-07-25), which set `lanes=2` on 3,956 ways across Monaco and
north-west Italy, one-way and two-way alike; later edits kept it. 67 one-way Monaco ways still carry `lanes=2` from it: each is a
candidate for the same error, but some one-way roads really have two lanes, so each is fixed only once checked. Street View shows one
lane. The sidewalks are separate `footway=sidewalk` lines (e.g. `1342546102`, about 3.3 m from the road line), so they
are not counted in `lanes`.

**Fix.** `lanes: 1` for each in `osm_overrides.yaml`; upstream, set `lanes=1` on these ways in OpenStreetMap.


## 11. Monaco, Avenue Albert II roundabout - no `lanes` tag on a two-lane ring

| field | value |
| --- | --- |
| **Area** | Monaco |
| **OSM ways** | `503462476`, `503462459`, `503462460`, `804900035`, `4229900` (`highway=secondary`, `junction=roundabout`, no `lanes`) |
| **Status** | **CONFIRMED by Street View** (2026-10-10); override enabled |

**Symptom.** The ring was drawn with one lane (the lane profile's default, `source = default`); the 2-lane road entering it
(`92627408`) had a lane with no way in (SUMO let it end before the ring).

**Fix.** `lanes: 2` for the five ways in `osm_overrides.yaml`; upstream, tag `lanes=2` on them in OpenStreetMap.

