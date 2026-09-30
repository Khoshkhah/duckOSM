# Global-junction segmentation (cross-mode `edge_id` alignment)

Config `options.global_junctions` (default on; falls back to per-mode junctions when off, and for
clip builds). It makes the same physical road carry the **same `edge_id`** in the driving, walking
and cycling graphs.

## The problem

`edge_id` is a content hash of a road segment; downstream tools (map matchers, per-edge
data tables) pin to it and JOIN across modes on it. That only works if a physical stretch of road is **segmented the same
way** in every mode — same split nodes → same geometry hash → same `edge_id`.

Today it is **not** always the same, so a single street fragments across modes.

### Worked example — Kalevi, Tartu

Kalevi is one street mapped as two OSM ways that meet at node `330040548`:

| osm_id | part | tags | driving | walking | cycling |
|---|---|---|---|---|---|
| `223203426` | two-way | `sidewalk=both` | 2 edges | 2 edges | 2 edges |
| `223470794` | one-way | `oneway=yes`, no sidewalk | 7 edges | **0 edges** | 6 edges |

The two-way part (`223203426`) has **identical `edge_id`s in all three modes** (`8712…`, `6814…`) — it
aligns. The one-way part (`223470794`) does **not**:

- **driving** splits it into 7 edges — e.g. `8611…` runs `6694401950 → 330040548` (44.6 m).
- **cycling** splits it into 6 edges — e.g. `5639…` runs `6694401945 → 330040548` (92.8 m), i.e. the
  driving `8611…` stretch **merged** with its neighbour into one longer edge with a **different hash**.
- **walking** has it **not at all** (a separate bug — see *Interaction with the walking hole*).

Because `8611…` (driving) and `5639…` (cycling) cover overlapping tarmac but hash differently,
`merge_modes` cannot unify them: the street renders as a patchwork of **"drive only"** and
**"cycle only"** fragments instead of one **"all 3"** road, and any cross-mode `edge_id` JOIN silently
drops these segments.

### Why it diverges

The split point is node `6694401950`, where a **service driveway** (`osm 711971148`,
`highway=service, service=driveway, access=private`) meets Kalevi.

`GraphSimplifier` already tries to keep segmentation stable across modes: a road splits **only at
*road* junctions** (`is_road_junction`), so a footway/path/cycleway joining a road mid-segment does
**not** fragment it (`_IS_ROAD` excludes `footway/path/cycleway/steps/pedestrian/bridleway/corridor`).

But `service` **is** a road for `_IS_ROAD`, so node `6694401950` is a road junction **wherever the
driveway is present**:

- **driving** includes the driveway → 2 road ways meet → road junction → Kalevi splits here.
- **cycling** drops the driveway (`access=private`, no `bicycle=yes`) → only Kalevi is present → **not**
  a road junction → Kalevi is **not** split here → merged edge, different `edge_id`.

**Root cause:** `is_road_junction` is computed **per mode**, from *that mode's already-filtered* ways.
When a road-class way is present in one mode but removed from another by an **access / oneway / bicycle
restriction** — or by a **mode-specific class list** (e.g. `tertiary` is admitted for driving/cycling
but not for the base walking filter) — the junction-ness of a shared node diverges, and any road
crossing that node is segmented differently, producing a per-mode `edge_id`.

The existing `_IS_ROAD` guard only covers the *non-road* case (paths). It does **not** cover a
road-class way that is *mode-filtered out*, which is exactly the Kalevi case.

## The fix — segment every mode at one global junction set

Decide junctions **once**, from the full raw road network (mode-agnostic), and reuse that same set when
segmenting each mode's ways.

1. **Global road set.** From `raw.ways`, take every way whose `highway` is a *road class in the union
   of all modes* — i.e. everything except `_NON_ROAD_HIGHWAYS`
   (`footway/path/cycleway/steps/pedestrian/bridleway/corridor`). This is independent of any per-mode
   `access` / `oneway` / `bicycle` / class filtering. A `service` driveway counts; a `tertiary` counts
   even though the base walking filter omits it.

2. **Global junctions.** Build `main.global_junctions(node_id, is_road_junction)` from the global road
   set exactly as `_find_junctions` does today, but over `raw` ways rather than a mode's filtered ways:
   a node is a **road junction** when ≥2 global-road ways meet there, or a global road ends there.
   (Non-road ways still contribute ordinary junctions for their own routing, unchanged.)

3. **Per-mode segmentation uses the global set.** `GraphSimplifier._segment_ways` splits a road at a
   node iff that node is a **global** road junction (`main.global_junctions.is_road_junction`), instead
   of the mode-local `junctions` table. Non-road ways keep splitting at every (mode-local) junction.

With this, Kalevi splits at node `6694401950` in **every** mode (globally, a road meets there), so the
`6694401950 → 330040548` stretch is its own edge with the **same geometry and `edge_id`** in driving
and cycling — they merge to "all 3".

### The degree-2 subtlety (important)

In cycling the driveway is absent, so after splitting Kalevi at `6694401950` **nothing branches there**
— the node is degree-2 within cycling. The current pipeline contracts degree-2 same-`osm_id` chains
(`GraphSimplifier` step 4b), which would **re-merge** the two cycling edges and undo the alignment.

So a **global road junction must be a hard split that survives contraction**: the degree-2 contractor
must treat `main.global_junctions.is_road_junction` nodes as non-contractible, even when locally
degree-2. Net effect: cycling/walking gain a few extra degree-2 nodes at driving-only junctions — a
benign cost that buys stable cross-mode `edge_id`s. (Routing is unaffected; an extra through-node on a
straight run changes nothing but the segmentation.)

## Interaction with the walking hole (separate bug)

Global junctions align *segmentation where a road exists in a mode*. They do **not** resurrect a road a
mode dropped entirely. Kalevi `223470794` is absent from walking because the **walking `RoadFilter`**
admits `tertiary` only when `sidewalk`/`foot` is tagged (`road_filter.py`), and this way is untagged.
That is a genuine pedestrian dead-end and should be fixed alongside, by broadening the walking filter to
include `tertiary`/`unclassified` (excluding `motorway`/`trunk` and `foot=no`), matching standard foot
profiles. Track as **P2**; this doc is **P1** (alignment). Both want the same rebuild.

## Alternatives considered

- **Status quo (`_IS_ROAD` per mode).** Keeps paths from fragmenting roads but not access/class-filtered
  road ways. Insufficient — the Kalevi case.
- **Post-hoc `edge_id` crosswalk.** Keep per-mode segmentation, add a table mapping
  driving↔cycling↔walking `edge_id`s by geometry overlap. No rebuild, but adds a brittle join layer
  (partial overlaps, 1-to-many) and every downstream consumer must learn it. Rejected.
- **Admit all road classes in every mode, mark non-routable instead of dropping.** Would also align
  junctions, but is a larger semantic change to each mode's graph (non-routable edges leak into
  routing/cost). Rejected in favour of the narrower junction fix.
- **Global-junction segmentation (this doc).** One mode-agnostic junction set; smallest change that
  makes `edge_id` a stable cross-mode key.

## Touch points

- `graph_simplifier.py` — `_find_junctions` (add/consume a global junction source), `_segment_ways`
  (split on global `is_road_junction`), degree-2 contractor (protect global road junctions).
- A pre-pass (in `GraphBuilder` or the importer, before per-mode simplification) that builds
  `main.global_junctions` from `raw.ways` + `raw.nodes`. Must run once, shared by all modes.
- `config.py` — a flag (e.g. `simplify.global_junctions: bool = True`) to gate/rollback the behaviour.

## Rebuild & verification

`edge_id`s change for affected roads (re-hash) → **full rebuild of all areas/modes** and re-run of any
downstream matchers pinned to `edge_id`. Verify:

1. `223470794` has matching `edge_id`s across driving & cycling (and walking once P2 lands).
2. `merge_modes` renders Kalevi as a single **"all 3"** street (no drive-only/cycle-only fragments).
3. Cross-mode edge counts: a road present in ≥2 modes shares `edge_id`s for every segment it has in
   common (spot-check a sample; assert 0 geometry-identical-but-different-`edge_id` pairs across modes).
4. Routing regression: shortest paths per mode unchanged vs. pre-fix on a fixed OD sample (extra
   degree-2 nodes must not change routes or costs).
