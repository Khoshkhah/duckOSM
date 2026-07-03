# Multimodal (intermodal) routing — design & build spec

> **Status:** ✅ **Phase 1 (v1 coarse) implemented.** walk ↔ drive ↔ cycle transfers, OSM-only,
> static Dijkstra. Build the `mm.*` tables with `duckosm multimodal <db>` (or `multimodal.enabled`
> in a build config) and route with `duckosm.route_multimodal`. The **v2 realistic** park-and-ride
> path (`--realistic`) is wired but stubbed (raises `NotImplementedError`); transit (Phase 3) is
> out of scope. The remaining sections below are the original design spec, kept as the rationale
> and the roadmap for v2/transit.

## Usage (Phase 1, v1 coarse)

Build the intermodal graph into a network that already has ≥2 modes including `walking`:

```bash
# add mm.edges + mm.transfers to an existing built db (writes the `mm` schema in place)
duckosm multimodal data/db/sodermalm.duckdb --transfer-cost 60

# or build it as part of a fresh build — in the YAML config:
#   modes: [driving, walking]
#   multimodal: {enabled: true, transfer_s: 60}
```

Then route a trip that switches mode (walk → drive → walk / park-and-ride):

```python
import duckdb
from duckosm import route_multimodal

con = duckdb.connect("data/db/sodermalm.duckdb")     # read-write only needed for the build step
r = route_multimodal(con, SRC_NODE, DST_NODE)        # OSM junction node_ids; default walking↔walking
r["time_s"]                                          # door-to-door seconds = Σ edge cost_s + Σ transfer cost_s
r["legs"]                                            # [{mode, edges, time_s, path}, …] grouped by mode
r["transfers"]                                       # the mode-change points [{node_id, from_mode, to_mode, cost_s, kind}]
route_multimodal(con, SRC, DST, start_mode="walking", end_mode="driving")   # e.g. end at a parked car
route_multimodal(con, SRC, DST, enforce_sequence=False)   # allow any mode alternation (default enforces walk* veh* walk*)
```

`route_multimodal` is a node-based Dijkstra over vertices `(node_id, mode)` (hand-rolled `heapq`, no
networkx needed). `enforce_sequence=True` (default) restricts a trip to `walk* (drive|cycle)* walk*`
— at most one contiguous vehicular segment, entered and left via walking.

**Visualise a route by mode** (walking green, driving red, cycling blue) with the sibling
[`mapstyle`](../../mapstyle) deck.gl viewer:

```bash
# auto-picks a walk->drive->walk trip across Sodermalm and renders it
python scripts/multimodal_route_map.py --db data/db/sodermalm_pbf.duckdb
python scripts/multimodal_route_map.py --db <db> --src <node_id> --dst <node_id> --out-dir reports/mm
python reports/<db-stem>_multimodal_route/serve.py     # then open the printed http URL
```

Implementation: `MultimodalBuilder` in `src/duckosm/processors/multimodal.py`, `route_multimodal` in
`src/duckosm/routing.py`, the `multimodal` CLI subcommand in `src/duckosm/cli.py`, config block in
`src/duckosm/config.py`, tests in `tests/test_multimodal.py`.

## Goal

Support trips that **change transport mode mid-route** — e.g. **walk → drive → walk**
(park-and-ride / "walk to the car, drive, walk from parking"), walk → cycle → walk, etc.

Today duckOSM builds three **independent single-mode graphs** (`driving`, `walking`, `cycling`),
each in its own schema, with **no arcs connecting them**. Routing (`duckosm.routing`) takes a single
`mode` argument, so a route stays in one mode end to end. A walk→drive→walk trip cannot be computed.

> **Not to be confused with [`multi_mode.md`](multi_mode.md).** "Multi-mode" = *building the three
> separate per-mode networks*. "Multimodal" (this doc) = *routing that switches between them* within
> one trip. This spec builds on top of the multi-mode networks.

## Current data model (what already exists)

Per build, the importer (`src/duckosm/importer.py`) loops over the configured modes and creates one
**schema per mode** (`CREATE SCHEMA IF NOT EXISTS {mode}`). Each mode schema contains:

| table | key columns | notes |
|---|---|---|
| `edges` | `edge_id`, `source`, `target`, `osm_id`, `highway`, `oneway`, `length_m`, **`cost_s`**, `maxspeed_kmh`, `geometry`, `is_reverse` | directed segments; `cost_s` = travel time in seconds |
| `nodes` | `node_id`, `geom` | **`node_id` = raw OSM node id** (`rn.osm_id AS node_id`) |
| `edge_graph` | `from_edge`, `to_edge`, `via_edge`, `cost` | **edge-based** adjacency (nodes = edge_ids); `cost` = `from_edge.cost_s`; turn restrictions already removed |
| `turn_restrictions` | … | per-mode |
| `ways`, `way_nodes` | … | intermediate |

Also: `main` schema (`boundary`, `admin_boundaries`, viz meta), `raw` schema (`raw.nodes`,
`raw.ways`, `raw.rels` from `ST_READOSM`).

### Four facts that make multimodal tractable

1. **`node_id` is the raw OSM node id**, so the *same physical junction* carries the *same*
   `node_id` in every mode's `nodes` table. Cross-mode linking is a plain equality join — no
   geometric snapping between mode graphs.
2. **Graph simplification preserves junctions.** `GraphSimplifier` contracts only degree-2
   through-nodes; junctions (degree ≠ 2) and endpoints survive in *every* mode. So real
   intersections — the sensible transfer points — reliably exist in both layers.
3. **`cost_s` (travel time in seconds) already exists per edge.** `SpeedProcessor` assigns
   per-mode speed (walking 5 km/h, cycling 15, driving per highway class), `cost.py` computes
   `cost_s = length / speed`. This is the common currency: transfer penalties in seconds add
   directly to travel cost. **Route on time, never on length** — modes have different speeds.
4. **`edge_id` is NOT unique across modes.** It is `(hash(osm_id, source, target, direction) >> 1)`
   (`graph_builder.py`) with **no mode input**, so the same physical way gets the *same* `edge_id`
   in driving/walking/cycling. ⇒ the multimodal edge key must be **`(mode, edge_id)`**.

## The model: a layered graph

Treat each mode as a **layer**. The routing vertex is **`(node_id, mode)`**.

- **Intra-mode arcs** = the mode's `edges`, weight `cost_s`. They never change `mode`.
- **Inter-mode arcs** = **transfers**, weight = a transfer penalty in seconds. They connect
  `(node_id, mode_A) → (node_id, mode_B)` at a shared node — the *only* arcs that change `mode`.

A multimodal route is then plain Dijkstra over (edges ∪ transfers), all weighted in seconds. This is
the standard layered / mode-expanded graph.

## Schema additions

Put everything in a new schema, `mm` (or `multimodal`), leaving the per-mode schemas untouched.

### 1. `mm.edges` — unified edges with a `mode` column

Union the per-mode `edges`, adding `mode`. Key is `(mode, edge_id)`. **It is a `VIEW`, not a
materialised table** — so it never duplicates the (potentially large) per-mode edge rows and always
reflects the current per-mode networks (rebuild a mode and `mm.edges` follows). A `name` column is
also carried through.

```sql
CREATE VIEW mm.edges AS
  SELECT 'driving'  AS mode, edge_id, source, target, cost_s, length_m, highway, name, geometry FROM driving.edges
  UNION ALL
  SELECT 'walking'  AS mode, edge_id, source, target, cost_s, length_m, highway, name, geometry FROM walking.edges
  UNION ALL
  SELECT 'cycling'  AS mode, edge_id, source, target, cost_s, length_m, highway, name, geometry FROM cycling.edges;
```

(Only union the modes actually present in the db — discover them the way `viz` does: schemas with an
`edges` table, excluding `main`/`raw`/`information_schema`/`pg_catalog`. Because `mm.edges` is a view,
it won't appear in `duckdb_tables()`; check `information_schema.tables` to test whether it exists.)

### 2. `mm.transfers` — the mode-change arcs (the essential new table)

```sql
CREATE TABLE mm.transfers (
    node_id    BIGINT,    -- shared OSM junction where the switch happens
    from_mode  VARCHAR,   -- e.g. 'walking'
    to_mode    VARCHAR,   -- e.g. 'driving'
    cost_s     DOUBLE,    -- transfer time/penalty in seconds (same unit as edges.cost_s)
    kind       VARCHAR    -- 'park' | 'retrieve' | 'bike_park' | 'bike_unpark' | 'trip_end'
);
```

Emit both directions where symmetric, or one row per direction (park vs retrieve can have different
costs). The router adds a `cost_s` transfer arc from `(node_id, from_mode)` to `(node_id, to_mode)`.

### 3. `mm.transfer_points` — catalog of *where* transfers are legal (+ new OSM categories)

This is the part that needs **new OSM data**. duckOSM currently extracts only *highway ways*; it
ignores the point features that say where a mode change is physically allowed. Without this you would
be teleporting into a parked car at any intersection. Extract these categories from raw OSM
(`raw.nodes` / `raw.ways` via `ST_READOSM`):

| OSM tag (category) | bridges modes | transfer `kind` |
|---|---|---|
| `amenity=parking`, `amenity=parking_entrance`, `park_ride=*` | walking ↔ driving | park / retrieve |
| `amenity=bicycle_parking` | walking ↔ cycling | bike_park / bike_unpark |
| `highway=bus_stop`, `railway=station`/`tram_stop`, `public_transport=platform`/`stop_position` | walking ↔ transit | board / alight — *needs GTFS; out of scope for v1* |

```sql
CREATE TABLE mm.transfer_points (
    point_id   BIGINT,      -- OSM node/way id of the amenity
    category   VARCHAR,     -- 'parking' | 'park_and_ride' | 'bicycle_parking' | ...
    modes      VARCHAR[],   -- ['walking','driving']
    geom       GEOMETRY,
    walk_node  BIGINT,      -- nearest node in walking.nodes (snapped)
    drive_node BIGINT       -- nearest node in driving.nodes (snapped)
);
```

`mm.transfers` is **generated** from this catalog: snap each point to the nearest node in each
relevant mode's `nodes`, then emit transfer rows. **Gotcha:** a parking amenity is usually a separate
OSM node *not on the road graph*, and its nearest walking node and nearest driving node may be
*different* physical nodes — so a realistic transfer may include a short connector, and `walk_node`
can differ from `drive_node`.

## Two fidelity levels — build the coarse one first

- **v1 — coarse (zero extra extraction).** Skip the catalog; make transfers from nodes the two
  graphs already share, with a flat penalty. Needs only `mm.edges` + `mm.transfers`.
  ```sql
  INSERT INTO mm.transfers
  SELECT w.node_id, 'walking', 'driving', 60.0, 'park'
  FROM (SELECT node_id FROM walking.nodes INTERSECT SELECT node_id FROM driving.nodes) w;
  -- + the reverse direction, + walking↔cycling, etc.
  ```
  Good for reachability / coverage analysis. **Unrealistic for trip planning** (you can grab a car at
  any shared junction). `log()`/document this limitation.
- **v2 — realistic park-and-ride.** Build `mm.transfer_points` from the OSM categories above, snap,
  and emit transfers **only** at those points (car ingress/egress restricted to parking). This is
  what makes walk→drive→walk trustworthy.

## Public transport (bus, train) — a separate, later phase, NOT a peer mode

Do **not** add `bus`/`train` as free-flow modes alongside walk/drive/cycle. They are a different
class and cannot be built the same way:

- **Free-flow modes (walk/drive/cycle):** traverse any edge at any time; cost = length/speed. Fully
  built from OSM; static Dijkstra. ← this spec.
- **Scheduled modes (bus/train):** you can only move along a **line** at **timetabled times**.
  Routing is **time-dependent** ("when is the next departure?"), needing a different algorithm
  (RAPTOR / Connection Scan / a time-expanded graph), not static Dijkstra.

Why they can't just be another mode filter:

1. **OSM has no timetables.** It has PT *infrastructure* (`highway=bus_stop`,
   `public_transport=platform`/`stop_position`, `railway=rail`/`tram`/`subway`/`station`,
   `route=bus`/`train` relations) but **no departure times** — you cannot route transit from OSM
   alone.
2. **The schedule lives in GTFS** (agency-published: stops, routes, trips, stop_times, calendar) —
   a new data source, not a new OSM filter. (pt2matsim does exactly this: OSM for the network, GTFS
   for the schedule.)
3. **Bus rides the road graph** — it runs on `driving` edges restricted to a route, stopping at
   stops; there is no separate "bus network" to build. Only **rail** has its own physical network
   (`railway=*`), which *can* be extracted from OSM like the road modes — but that is for rail
   *simulation* (railML/OpenTrack, see export targets), not journey planning.

Sequencing:

- **Phase 1 (this spec):** walk ↔ drive ↔ cycle. Static, OSM-only. Build first.
- **Phase 2 (optional):** a `rail` **network** mode from OSM `railway=*` — for simulation, not
  journey planning. Structurally like the road modes.
- **Phase 3 (separate, bigger):** transit **journey planning** — ingest **GTFS**, add stops as
  walk↔transit transfer points, add a **time-dependent** router (RAPTOR/CSA). Here bus and train
  become routable.

**Forward-compatibility:** the layered `(node_id, mode)` model still holds — transit is another
layer, stops are transfer points — **but** its arcs are time-dependent, so it needs a time-expanded
representation or a RAPTOR-style algorithm alongside the static Dijkstra. Keep `mm.transfers` and the
router open to a transit layer; don't build it now.

## Build plan (implementation steps)

1. **New processor `MultimodalBuilder`** (`src/duckosm/processors/multimodal.py`), registered in
   `processors/__init__.py`. Runs *after* all per-mode builds complete. Creates schema `mm`, builds
   `mm.edges` (union + `mode`).
2. **Coarse transfers (v1):** `mm.transfers` from the shared-node INTERSECT + configurable flat
   penalty per (from_mode, to_mode).
3. **`TransferPointExtractor` (v2):** pull `amenity=parking`/`park_ride`/`bicycle_parking` points
   from raw OSM into `mm.transfer_points`; snap each to nearest node per relevant mode
   (`ST_Distance` / nearest-neighbour on `nodes.geom`); regenerate `mm.transfers` from the catalog.
4. **Mode-change rules:** enforce a valid mode sequence in the router — a legal trip is
   `walk* (drive|cycle)* walk*`, not arbitrary alternation. Simplest: the router only *starts* and
   *ends* in `walking`, and allows at most one drive/cycle segment (configurable).
5. **Multimodal router** (`routing.py`, e.g. `route_multimodal(con, src_node, dst_node, ...)`):
   Dijkstra over vertices `(node_id, mode)`; neighbours = same-mode `mm.edges` out of `node_id`
   **plus** `mm.transfers` out of `(node_id, mode)`. Weight = `cost_s`. Return the leg-by-leg path
   with mode and the transfer nodes.
   - *Node-based* is simplest (vertices `(node_id, mode)`). If you need turn-restriction fidelity
     across the whole trip, use the *edge-based* variant instead: express each transfer as edge→edge
     arcs (every in-edge of the node in mode A → every out-edge in mode B, + `cost_s`), and route on
     a unified `edge_graph`. Start node-based.
6. **CLI:** `duckosm multimodal <db> [--transfer-cost 60] [--realistic]` builds the `mm.*` tables
   into an existing db (mirror how `admin` / `sumo` wrap a built db).
7. **Config:** add `options.multimodal` (bool), default transfer penalties per mode pair, and the
   category list — follow the pattern in `config/default.yaml` / `src/duckosm/config.py`.
8. **Optional exports:** a unified `mm.edges` also unlocks *multimodal* SUMO export (SUMO simulates
   mixed traffic) and a `mode`-tagged GMNS — see [simulation export targets in the product notes].

## Key decisions & gotchas (don't relearn these)

- **Key on `(mode, edge_id)`** — `edge_id` collides across modes (no mode in the hash).
- **Vertex is `(node_id, mode)`** — the layered-graph identity.
- **Route on `cost_s` (seconds)** — already present; transfers are in seconds so they add cleanly.
- **POI snapping:** parking points aren't on the road graph; `walk_node` ≠ `drive_node` is normal.
- **Transfer directionality:** park (walk→drive) and retrieve (drive→walk) can carry different
  costs; emit per-direction rows.
- **Valid mode sequence:** enforce `walk* drive* walk*` (a car trip is bracketed by walking legs) —
  otherwise the router will "teleport" between cars.
- **Simplification helps you:** junctions survive in every mode, so shared `node_id`s for transfers
  are reliably present.
- **Turn restrictions:** only preserved end-to-end if you go edge-based (step 5 variant).

## Testing (mirror `tests/test_road_filter_oneway.py` style)

Build a tiny two-mode fixture (a `walking` and a `driving` schema sharing one junction `node_id`) and
assert:
1. A walk→drive→walk path exists across the shared node and its cost includes the transfer penalty.
2. Without any `mm.transfers` row, no path changes mode (the layers are disconnected).
3. With v2 restrictions, a transfer exists **only** at a parking node, not at an arbitrary junction.
4. Route weight is in seconds and matches `sum(edge cost_s) + sum(transfer cost_s)`.

## Integration points (files to touch)

- `src/duckosm/processors/multimodal.py` — new `MultimodalBuilder` (+ `TransferPointExtractor`).
- `src/duckosm/processors/__init__.py` — register.
- `src/duckosm/importer.py` — optional post-mode step guarded by `config.options.multimodal`.
- `src/duckosm/routing.py` — `route_multimodal(...)` (+ maybe a `to_networkx_multimodal`).
- `src/duckosm/cli.py` — `multimodal` subcommand.
- `src/duckosm/config.py`, `config/default.yaml` — `options.multimodal`, transfer costs, categories.
- `tests/test_multimodal.py` — the cases above.

## Related

- One-way is mode-specific — see `_oneway_expression()` in `road_filter.py` and
  `tests/test_road_filter_oneway.py` (walking is undirected; driving/cycling directed).
- Node-based vs edge-based graphs — `to_networkx_nodes()` vs `to_networkx()` in `routing.py`.
