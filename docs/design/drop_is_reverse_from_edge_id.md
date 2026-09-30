# Removing `is_reverse` from the edge_id hash

**Status:** implemented (branch `edge-id-segmentation-overhaul`). Touches core segmentation
(`GraphSimplifier` / `GraphBuilder`) and the `edge_id` definition, so it changes ids in every DB
and requires a **full rebuild of all areas + downstream re-match** (handled separately). The old
formula is retained as the `edge_id_hash_v1` macro for old→new crosswalks.

## Summary

Today `edge_id = (hash(osm_id, source, target, is_reverse) >> 1)`. We want to drop `is_reverse` from
the hash so `edge_id = (hash(osm_id, source, target) >> 1)`. That is only safe if
`(osm_id, source, target)` is **globally unique**. It currently is **not** — a small number of
self-crossing ways break it — so this doc specifies a geometry-preserving fix that makes the triple
unique, after which `is_reverse` can leave the hash (it stays as a column).

## Where this sits in the roadmap

This edge_id change is one of four coordinated steps. `edge_id` is a pure function of the final
segmentation, so segmentation must be settled before ids lock:

| # | step | doc | changes |
|---|---|---|---|
| 3 | mode-agnostic junctions → consistent **merging** across modes | `global_junction_segmentation.md` | split/merge points → `source`/`target` |
| 4 | two virtual nodes on every loop, **all modes** | this doc | `source`/`target`, +nodes |
| 1 | new `edge_id` (drop `is_reverse`) + readable `edge_ref` | this doc | `edge_id`, `edge_ref` |
| 2 | `walk_type` / `cycle_type` columns | `walk_cycle_type.md` | attribute columns (independent) |

**Dependency:** steps 3 and 4 change `source`/`target`, and step 1's `edge_id`/`edge_ref` are computed
from them — so implement **3 → 4 → 1** (the re-key runs last in the pipeline anyway); step 2 is
independent. Step 4's loop split runs inside `GraphSimplifier`, which already executes per mode, so it
covers driving / walking / cycling automatically.

## Why `is_reverse` is in the hash today

Not for direction — **direction is already given by `source → target`.** Its *only* job in the hash
is a **uniqueness tiebreaker** for the rare case where two genuinely different edges share the same
directed `(osm_id, source, target)`. Example (`sodermalm_pbf`, `osm_id 141633691`, a `service` road):

```
edge 4876567021791941776:  1550340400 → 1550340390,  is_reverse=False,  49.9 m   (long arc)
edge 7138719905413293304:  1550340400 → 1550340390,  is_reverse=True,   19.3 m   (short link, reversed)
```

Same osm_id, same source→target, different geometry and length — two different pieces of a lollipop
loop. `source/target` give direction but **not identity**; only `is_reverse` tells these apart.

## Collision analysis (measured)

Dropping `is_reverse` naively collides in **every DB and mode — 64,772 duplicate ids total**:

| db | driving | walking | cycling |
|---|--:|--:|--:|
| burnaby | 170 | – | – |
| sodermalm_pbf | 12 | 26 | 12 |
| tartu | 158 | 182 | 152 |
| vancouver_city | 132 | 228 | 102 |
| stockholm_county | 4,172 | – | – |
| estonia | 4,836 | 5,222 | 6,264 |
| sweden | 42,590 | – | – |

Self-loops (`source = target`) are **0** everywhere — those are already split at a virtual midpoint
(`_split_self_loops`). The collisions are a different phenomenon (below).

## Root cause: antiparallel forward pairs + the reverse step

Direction (the reverse twins) is added **after** splitting (`_add_reverse_edges` runs on the
already-split forward set). Two facts follow, both measured:

- **Among forward edges alone, `(osm_id, source, target)` is already unique — 0 collisions.**
- The collision is created entirely by the reverse step, and only for a way with an **antiparallel
  forward pair**: both `A→B` and `B→A` present as forward edges. Adding reverses then yields
  `reverse(B→A) = A→B` colliding with the forward `A→B` (and symmetrically).

It bites **two-way** ways only (one-way ways get no reverse twins, so they never collide even when
self-crossing). The arithmetic is exact:

| | forward collisions | antiparallel fwd pairs | of which two-way | post-reverse collisions |
|---|--:|--:|--:|--:|
| burnaby driving | 0 | 108 | 85 | 170 (= 2 × 85) |
| sodermalm driving | 0 | 12 | 6 | 12 (= 2 × 6) |
| sodermalm walking | 0 | 18 | 13 | 26 (= 2 × 13) |

Two sources produce antiparallel forward pairs, and both must be handled:

1. **Self-loops** — after `A→A` is split into `A→M` + `M→A`, those two halves *are* an antiparallel
   pair. (This is why the self-loop split is the majority of the collisions: ~60–77% involve a
   virtual node.)
2. **Doubling-back / self-crossing ways** — a way whose node list revisits a junction pair, e.g.
   `osm_id 141633691` (node `1550340400` appears twice), giving a long arc and a short link between
   the same two junctions.

## The fix: geometry-preserving virtual nodes

**Requirement: every part keeps its own geometry** — each edge holds a complete `LINESTRING`, and the
long arc is real road, not redundant geometry. So we make the triple unique by **splitting** an arc,
not deleting one. Inserting a virtual node `M` splits an arc's geometry in two without losing any:

```
before:  A ──long arc L──► B        +   A ◄──short arc S── B      (antiparallel → collides)
insert M at the midpoint of the longer arc:
after:   A ──L₁──► M ──L₂──► B       +   A ◄──S── B
         (L₁ + L₂ == L via ST_LineSubstring; nothing lost)
```

The forward edges become `A→M`, `M→B`, `B→A`; after reverses every directed pair is distinct — **no
collision, zero geometry lost.** Self-loops fold into the same idea as a **3-way split**
(`A→M1→M2→A`, two virtual nodes), which keeps the entire loop geometry.

### Detection (runs on the forward set, before `_add_reverse_edges`)

For each **two-way** `osm_id`, find antiparallel forward pairs (`A→B` and `B→A`). For each pair,
insert a virtual node at the midpoint of the **longer** arc and split it. Self-loops are the special
case where the antiparallel pair is `A→M` / `M→A`; give them the 3-way split.

New virtual-node ids extend the existing negative scheme
`-((hash(osm_id, source) >> 2))` with a discriminator so the two midpoints are deterministic and
distinct (e.g. `hash(osm_id, source, 1)` / `hash(osm_id, source, 2)`).

### Rejected alternative: drop the longer arc

We considered extending the existing same-direction dedup ("keep shortest") to antiparallel pairs —
delete the long arc, keep the short. It is **routing-lossless** on a two-way road (the short link
connects A↔B both ways; the long arc is never optimal) and simpler. **Rejected** because it deletes
real, distinct geometry, and the design requirement is that every part keeps its geometry. Recorded
here so the trade-off is explicit: if a future consumer only needs routing and not geometry, this is
the cheaper path.

### One-way ways

Left untouched. A one-way self-crossing way's two arcs are *different* directed connections (`A→B`
long is the only A→B path; `B→A` short is the only B→A path), so neither may be removed **and** they
never collide (no reverse twins). No fix needed or wanted.

## `is_reverse`: drop from the hash, keep as a column

- **Hash:** remove it — once `(osm_id, source, target)` is unique, `hash(osm_id, source, target)` is
  collision-free.
- **Column:** keep it. It still carries "along vs. against OSM digitization," which routes
  `lanes_fwd`/`lanes_bwd` and other directional tags. Dropping it from the hash costs nothing here.

## Id representation — an additional readable column

Alongside the compact integer `edge_id`, add a **secondary, human-readable** id `edge_ref`:

```
edge_ref = {osm_id}#{seq}{dir}          dir ∈ { f, r }
```

| edge | `edge_ref` |
|---|---|
| way 41790239, segment 2, forward | `41790239#2f` |
| way 41790239, segment 2, reverse | `41790239#2r` |
| way 41790239, segment 0, forward | `41790239#0f` |

Design choices:

- **`osm_id` leads** so all edges of a way sort/group together, and the id is self-describing and
  **parseable back** to `(osm_id, seq, dir)` — the reversibility the hash lacks.
- **Direction is an explicit `f`/`r` suffix, not a sign.** A leading `+`/`-` is rejected: `-41790239`
  parses as a *negative integer*, which already means "virtual node" (`node_id < 0`) in this schema;
  `+` gets silently stripped by many tools; and a sign prefix scatters a way's two directions when
  sorting. Direction is a category, so encode it as a token.
- **`seq` is the segment index within the way** (`0…N-1`, traversal order). It disambiguates the
  doubling-back case for free (the two arcs of a lollipop are different segments → different `seq`),
  so `edge_ref` is **unique by construction** and never relies on a hash. Optional: zero-pad
  (`#02f`) if lexicographic sort must match numeric order.
- Delimited variant `{osm_id}:{seq}:{dir}` if fully-unambiguous parsing is preferred over compactness.

**`edge_ref` is a secondary column, not the join key.** The integer `edge_id` (content hash) stays
the fast, stable key across `edge_graph` / matchers / downstream; `edge_ref` is for
human/debug use. This split matters because `seq` is **positional → not rebuild-stable** (insert or
remove an edge and later `seq`s renumber). That instability is acceptable for a readable label but
would be wrong for a join key — which is exactly why `edge_id` remains the hash and `edge_ref` rides
alongside it.

## Implementation plan

1. `GraphSimplifier`: add an antiparallel-pair split step on the forward edges, **before** the
   reverse-edge step; fold `_split_self_loops` into it as the 3-way case.
2. Change the hash in `_rekey_edges`, `GraphBuilder`, and the reverse-edge builder to
   `(hash(osm_id, source, target) >> 1)::BIGINT`; keep the `is_reverse` column.
3. Add the `edge_ref` column: assign a per-`osm_id` `seq` with a window function over the forward
   edges in traversal order (`ROW_NUMBER() OVER (PARTITION BY osm_id ORDER BY …)`), carry the same
   `seq` to each reverse twin, then build `osm_id || '#' || seq || (is_reverse ? 'r' : 'f')`.
4. Assertions after re-key: `count(*) == count(distinct edge_id)` **and**
   `count(*) == count(distinct(osm_id, source, target))` **and** `count(*) == count(distinct edge_ref)`.
5. Rebuild all DBs; run the collision check (expect 0 in every DB/mode).
6. Add `edge_id` (unchanged) and `edge_ref` (new) rows to `docs/data_dictionary.md`.
7. Downstream re-match (see below).

## Downstream impact

Edge ids for affected loops/doubling-back ways change, and new virtual nodes/edges appear. As with
the previous `edge_id` hash migration, the matchers keyed on `edge_id` must re-run:
`fetching-sweden-data` (route/flow matchers) and `traffic_tube_measurements_Stockholm`. Node and edge
counts rise slightly (one extra node + one extra edge per doubling-back pair; two per self-loop).

## Verification

- Per DB/mode: `count(*) == count(distinct(osm_id, source, target))` (0 collisions).
- `count(*) == count(distinct edge_id)` (the existing guard in `_rekey_edges`).
- Geometry preserved: total edge length per `osm_id` unchanged before/after the split step.
- Routing smoke test: a route across a former lollipop/self-loop still solves.

## Open questions

- Discriminator scheme for the two self-loop midpoints — confirm determinism/stability across
  rebuilds (must not depend on row-number ids).
- Should the antiparallel split use the midpoint (`0.5`) or the true self-crossing node when one
  exists? Midpoint is simplest and keeps `L₁ + L₂ == L`.
- Rare case: three+ arcs between the same pair (two same-direction handled by existing dedup, plus an
  antiparallel) — confirm the combined dedup + split leaves the triple unique.
