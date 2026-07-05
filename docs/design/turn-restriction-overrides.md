# Design: turn-restriction overrides (synthetic OSM restrictions)

**Status:** implemented & verified
**Related:** `docs/known_osm_issues.md` #6, `config/osm_overrides.yaml`, `processors/restrictions.py`,
`processors/edge_graph.py`

## Problem

Some junctions are missing an OSM turn-restriction relation that physically exists, so duckOSM
generates a **movement that is forbidden on the ground**. Concrete case (Tartu, node `330045016` —
known_osm_issues #6): the one-way link `2048307190992484263` (OSM way `1307524008`) gets a movement
onto the **opposing** Tähe carriageway `5604449516676458023` (OSM way `997402723`) — a −166° turn back
onto oncoming traffic, typed `uturn`. There is a legitimate `thru` from the same link, so the U-turn is
spurious. It pollutes **routing, the GMNS `movement` table, the lane graph, MATSim/OpenDRIVE/SUMO
exports, and the lanestyle lane-connectivity view** — everywhere downstream, not just the map.

The existing `osm_overrides.yaml` only patches **way** fields (`oneway` / `lanes` / `layer`). A
forbidden turn is a `from → via → to` triple, which no way-field can express.

## Key insight — reuse the rails duckOSM already has

duckOSM already models OSM turn restrictions end to end:

- `RestrictionProcessor` reads `type=restriction` relations → `restrictions_pivoted(restriction_id,
  restriction_type, from_way, via_node, to_way)` → maps to edges (`_map_to_edges`, correct for
  merged/reverse edges) → **`turn_restrictions(from_edge_id, via_node, to_edge_id)`**.
- `EdgeGraphBuilder` drops the forbidden transition: `no_*` removes `from → to`; `only_*` keeps only the
  mandated `to`.

So a **synthetic restriction** injected into the same pipeline needs no new consumer — the edge graph,
routing, GMNS movements, exports, and the viz all honour it automatically.

## Proposed change

### 1. `config/osm_overrides.yaml` — new `turn_restrictions:` list

```yaml
turn_restrictions:
  - from_way: 1307524008     # Tartu — no U-turn onto opposing Tähe carriageway (known_osm_issues #6)
    via_node: 330045016
    to_way:   997402723
    restriction: no_u_turn
    note: "OSM lacks the restriction; the -166° turn onto the opposing carriageway is forbidden"
```

Fields: `from_way`, `via_node`, `to_way` (OSM ids, required), `restriction` (any OSM value the edge
graph understands — `no_u_turn`, `no_left_turn`, `no_right_turn`, `no_straight_on`, `only_*`; default
`no_u_turn`), `note`. Like the way rules, each is a **global no-op** in areas that don't contain the
ways.

### 2. `RestrictionProcessor` — inject the synthetic rules

Give it `overrides_path` (as `OsmOverrides` already takes). After `restrictions_pivoted` is built from
OSM relations, **UNION the synthetic rules** into it (with a synthetic negative `restriction_id` so it
can't collide with a real relation id), then let the existing `_map_to_edges` map them to edge ids.
Because they flow through the same mapping, merged and reverse edges are handled identically to real
restrictions. Guard: ensure `restrictions_pivoted` exists even when the area has zero OSM restriction
relations (create it empty, then union).

### 3. `importer.py` — pass the path

Line ~614: `RestrictionProcessor(self.con, self.config.osm_overrides).run()`. Runs in the driving
`extract_restrictions` step, before `EdgeGraphBuilder` — no ordering change.

## Impact & scope

Removes the forbidden transition from `edge_graph`, so it disappears from routing, `gmns_*.movement`,
the lane graph / `route-lanes`, MATSim/OpenDRIVE/SUMO exports, and lanestyle — one fix, consistent
everywhere. Additive: areas without these ways are unaffected; behaviour with no `turn_restrictions:`
section is unchanged.

## Verification (done — re-ran `RestrictionProcessor` + `EdgeGraphBuilder` on Tartu)

- `RestrictionProcessor` injected the synthetic rule and mapped it to edges:
  `turn_restrictions` 69 → 70, row `('no_u_turn', from=2048307190992484263, to=5604449516676458023,
  via=330045016)`.
- `EdgeGraphBuilder` then **removed** the `2048307190992484263 → 5604449516676458023` transition
  (present 1 → 0) while **keeping** the legitimate `thru` `2048307190992484263 → 8788328652948037784`
  (present = 1).
- On the next full driving rebuild this propagates to `gmns_driving.movement`, the lane graph /
  `route-lanes`, exports, and the lanestyle view (`lane_adjacency` no longer lists it at all — genuinely
  forbidden, not merely reclassified).

## Alternatives considered

- **Way-field override** (`oneway`/`lanes`): can't express a turn; rejected.
- **Drop all `uturn` movements globally** (earlier option): over-broad — real, legal U-turns exist;
  a per-junction restriction is precise and matches how OSM models the ground truth.
- **Filter in lanestyle only**: leaves routing/exports wrong; rejected (the data should be right at the
  source).
