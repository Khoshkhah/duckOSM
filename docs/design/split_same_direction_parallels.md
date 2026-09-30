# Splitting same-direction parallel arcs (stop deleting lollipop geometry)

**Status:** implemented (2026-07-06, branch `edge-id-segmentation-overhaul`; same day as proposed).
`GraphSimplifier`: `_split_parallel_pairs` + the shared `_split_arcs_at_midpoint` helper, refs-salted
virtual ids, `_rekey_edges` raises on residual duplicates; `validate.py`: `unique_node_id` +
`way_length_conserved`; tests in `tests/test_edge_identity.py`. Verified on Burnaby (the worked
example below now keeps all 280 m across 10 edges and 2 distinct virtual nodes; detector reports 0
dropped arcs) and Tartu (all modes, all validators green). Virtual-node ids changed, so this landed
**before** the fleet rebuild for `drop_is_reverse_from_edge_id.md` — ids change once, together.

## Summary

`_rekey_edges` de-duplicates on `(osm_id, source, target)` keeping the **shortest** arc — the one
remaining place where the pipeline **deletes real geometry** instead of splitting it with a virtual
node. Self-loops and antiparallel pairs already get the geometry-preserving treatment
(`_split_self_loops` / `_split_antiparallel_pairs`); same-direction parallels — two arcs of one way
that connect the same junction pair in the same direction — are silently dropped. The drop is
invisible: no log line, and no validation invariant catches it.

Worse, the interaction with `_split_antiparallel_pairs` is buggy on the current branch: when a way
carries **two** arcs `A→B` plus one `B→A` (a figure-8 / double lollipop), *both* `A→B` arcs are
antiparallel to the `B→A` arc, both get split — **at the same virtual-node id** with two different
midpoint geometries. The nodes table ends up with a duplicate `node_id`, and the dedup then deletes
the second arc's halves anyway.

This doc specifies: (1) a geometry-preserving split for same-direction parallels, (2) an arc-unique
virtual-node id, (3) the dedup demoted to a logged safety net, (4) two new validation invariants so
this class of loss can never be silent again.

## Worked example — Burnaby, way `586121493` (measured)

A `service`/`parking_aisle` way whose node list is a **figure-8**: it visits junction
`A = 5600831376` at positions 1 and 10, and `B = 5600831373` at positions 7 and 17:

```
refs:  A ─(5 nodes)─ B ─(2 nodes)─ A ─(6 nodes)─ B
arcs:  ① A→B  100.5 m      ② B→A  75.1 m      ③ A→B  104.5 m
```

Arcs ① and ③ are a **same-direction parallel pair** (both `A→B`); each is also antiparallel to ②.

**Old-scheme build (`data/db/burnaby.duckdb`, pre-overhaul):** the dedup keeps ① and ②, deletes ③
whole — 104.5 m of mapped road (37 % of the way) gone from `edges`, so it is missing from every
render, every export, and unroutable.

**Current-branch build (verified by rebuilding Burnaby on `edge-id-segmentation-overhaul`):**

```
kept:  586121493#1f  A → V   50.2 m      V = -3197755042082330808
       586121493#3f  V → B   50.2 m
       586121493#2f  B → A   75.1 m      (+ the three reverse twins)
lost:  arc ③ entirely (forward total 175.5 m vs 280.1 m raw)
nodes: node_id -3197755042082330808 appears TWICE —
       POINT(-122.95551 49.20675)   (midpoint of ①)
       POINT(-122.95595 49.20687)   (midpoint of ③)
```

`_ap` selected both ① and ③ (each longer than ②), both were split at
`vnid = -(hash(osm_id, source, target) >> 2)` — identical inputs, identical id, two geometries. The
rekey dedup then kept the shorter half in each `(osm_id, A, V)` / `(osm_id, V, B)` group, deleting
③'s halves. Net: same geometry loss as before **plus** a duplicate-id virtual node that `validate`
does not flag.

## Measured scale

Post-hoc detector (a run of way refs covered by no kept edge, bounded by a node pair that a kept
edge of the same way connects — the exact signature of a dedup drop):

| build | mode(s) | dropped arcs | lost metres |
|---|---|--:|--:|
| burnaby (old build **and** branch rebuild) | driving | 1 | 104.5 |
| sodermalm_pbf | driving / walking / cycling | 0 | 0 |
| tartu | driving / walking / cycling | 0 | 0 |
| vancouver_city | driving / walking / cycling | 0 | 0 |

Rare — a way must revisit the *same junction pair in the same order* (figure-8s, double lollipops,
spiral service ways) — but not exotic: one suburb already has one, and country-scale builds
(sweden, estonia: not yet measurable, pre-overhaul schema) can be expected to carry tens. Rarity is
not a defence when the failure is silent deletion of mapped road.

## Root cause

Two independent decisions compose into the loss:

1. **The dedup is keep-shortest by design** (`_rekey_edges`): "the longer is never the optimal
   route between the two junctions". True for *through*-routing only — it ignores destinations
   **on** the arc, rendering, and exports. `drop_is_reverse_from_edge_id.md` already set the
   requirement "**every part keeps its own geometry**" and rejected keep-shortest for antiparallel
   pairs on exactly that ground; same-direction parallels were left as the acknowledged exception
   (its §Rejected-alternative and final open question).
2. **The virtual-node id is not arc-unique** (`_split_antiparallel_pairs`):
   `-(hash(osm_id, source, target) >> 2)` collides whenever two *distinct arcs* share endpoints and
   direction — precisely the same-direction case.

## The fix

### 1. `_split_parallel_pairs` — same-direction split, before the antiparallel split

On the forward set (same position in the pipeline as the existing splits): group forward edges by
`(osm_id, source, target)`; in every group of n ≥ 2, keep the **shortest** arc whole and split each
of the other n−1 arcs at its 50 %-by-length midpoint with a virtual node — the exact
`ST_LineSubstring` machinery `_split_antiparallel_pairs` already uses (factor the split into a
shared helper rather than copying it).

Applies **regardless of `oneway`**: two same-direction forwards collide with *each other* directly
— no reverse twin needed — so one-way ways are affected too (unlike the antiparallel case, which
only bites two-way ways).

Ordering: same-direction split **first**, then antiparallel. After it, at most one whole arc per
directed pair remains, so `_ap` degenerates to the plain two-arc case it was written for. For the
Burnaby figure-8: ③ splits at `V₃`, then ① (antiparallel to ②) splits at `V₁` — six forward edges,
all triples unique, 280.1 m fully preserved, `A↔B` routable through three distinct arcs.

### 2. Arc-unique virtual-node ids (fixes the duplicate-node bug)

Salt the virtual id with the arc's content, not just its endpoints, in **both** split steps:

```
vnid = -(hash(osm_id, source, target, refs::VARCHAR) >> 2)
```

`refs` (the arc's full node list) is deterministic across rebuilds, distinct between two arcs by
definition (same refs ⇒ same arc), and independent of row-number ids — the same stability argument
as the `edge_id` content hash. This changes existing antiparallel/self-loop vnids, which is why
this doc must land before the `drop_is_reverse` fleet rebuild: downstream re-matches once, not
twice.

### 3. Demote the dedup to a logged safety net

The keep-shortest dedup in `_rekey_edges` stays as defence-in-depth but must now match **zero**
rows. Log the removed count; the existing `n == distinct(edge_id) == distinct(osm_id, source,
target) == distinct(edge_ref)` assertion already guards uniqueness — add the dedup count to it so a
non-zero count fails loudly instead of deleting silently.

### 4. Two new validation invariants (`validate.py`)

- **`unique_node_id`** — `count(*) == count(distinct node_id)` in `nodes`. Would have caught the
  duplicate-vnid bug on day one.
- **`way_length_conserved`** — per `osm_id`, the sum of forward-edge `length_m` matches the raw
  way's ref-chain length (haversine over `raw.nodes`), for ways fully inside the boundary and not
  touched by `ComponentFilter`/`PathConnector`. Any future geometry-deleting change trips it.

## Rejected alternatives

- **Status quo (keep-shortest).** Routing-lossless for through-traffic, simplest — rejected: it
  deletes mapped road from renders/exports, strands destinations on the arc, and contradicts the
  geometry-preservation requirement this pipeline already adopted for the two sibling cases.
- **Side table of dropped geometry.** Keeps `edges` clean, but every renderer/export would need to
  union two tables; anything reading `edges` (roadstyle, mapstyle, GMNS, matchers) still sees the
  hole. Geometry belongs in `edges`.
- **Keep the arc flagged non-routable.** Leaves two rows with the same `(osm_id, source, target)` —
  needs an `is_reverse`-style hash tiebreaker again, undoing `drop_is_reverse_from_edge_id.md`.
- **Split at the true self-crossing node instead of the midpoint.** The crossing node is already a
  junction (both arcs end there); the collision is *between* junctions, where there is no natural
  split point. Midpoint keeps `L₁ + L₂ = L` and matches the existing splits.

## Downstream impact

For DBs built on this branch: the previously-deleted arcs come back as **new** edges/virtual nodes;
the kept shortest arc's endpoints are untouched. The vnid re-salt (fix 2) changes ids of
already-split antiparallel/self-loop arcs — bundled into the one `drop_is_reverse` fleet rebuild +
downstream re-match (`fetching-sweden-data`, `traffic_tube_measurements_Stockholm`) that is already
planned. Node/edge counts rise by one node + one edge per split arc.

## Implementation plan

1. Factor the midpoint-split SQL out of `_split_antiparallel_pairs` into a helper
   (select-arcs → split-at-0.5 → rebuild forward table → register virtual nodes).
2. Add `_split_parallel_pairs` (same-direction groups, keep-shortest-whole, split the rest) and
   call it before `_split_antiparallel_pairs` in `run()`.
3. Re-salt `vnid` with `refs::VARCHAR` in both splits (and the self-loop discriminators).
4. `_rekey_edges`: log the dedup count and fail the assertion when non-zero.
5. `validate.py`: add `unique_node_id` and `way_length_conserved`.
6. Tests: a synthetic figure-8 fixture (refs `A…B…A…B`, two-way) and a one-way same-direction
   variant — assert total forward length equals raw length, all ids unique, no duplicate nodes,
   and a route can traverse each arc.
7. Rebuild Burnaby; re-run the post-hoc detector (expect 0 dropped arcs) and the two new
   invariants across all areas.

## Verification

- Detector run on rebuilt areas: `dropped_arcs = 0`, `lost_m = 0` everywhere. ✔ (Burnaby, Tartu)
- Way `586121493`: forward length 279.9 m (= 280.1 m raw minus half-length rounding), five forward
  edges + five reverse twins, two distinct virtual nodes, none duplicated. ✔
- Existing suites stay green (`test_merge_segments`, `test_routing`, uniqueness assertions). ✔
  (147 tests pass, incl. the 8 new ones)

## Open questions (resolved at implementation)

- Groups of n ≥ 3 same-direction arcs: the selection is pairwise ("longer than any other arc of
  the group", refs tiebreak on equal length), which selects all but the shortest — each split arc
  gets a distinct refs-salted virtual id, so uniqueness holds by construction and the `_rekey_edges`
  guard would fail the build otherwise. Truly *identical* twin arcs (same refs) would still raise —
  acceptable, they have never been observed and warrant a look before any auto-dedup.
- `way_length_conserved` runs per mode, implemented **topologically** (a missing interior ref run
  bounded by a kept edge of the same way bridging its endpoints) rather than by metre comparison —
  immune to clip/component/PathConnector confounders, no tolerance needed. Skipped on clip builds
  (no per-mode `ways` table).
- `refs::VARCHAR` salt kept as specified; the equal-length tiebreaks in BOTH split selections were
  also moved from the volatile row-number `edge_id` to `refs`, making the *choice* of split arc
  rebuild-stable too.
