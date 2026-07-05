# MATSim network export → `duckosm matsim` (design)

**Status:** **shipped** 2026-07-04 (`to_matsim` / `duckosm matsim`, single-mode v1). Tier 1, item 2 on
the [export roadmap](../../product/simulation-export-targets.md) — the next target after
[GMNS](gmns_export.md). Emits a MATSim **`network.xml`** from a built duckOSM routing db, plugging
duckOSM into the MATSim / **BEAM** / eqasim agent-based ecosystem.

## What MATSim's network is

A MATSim network is deliberately simple: a directed graph of **nodes** + **links**, one XML file
(conventionally gzipped, `network.xml.gz`), declared against the `network_v2` DTD. It is the
*substrate* MATSim (and BEAM, which reads MATSim inputs directly) runs agents on.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE network SYSTEM "http://www.matsim.org/files/dtd/network_v2.dtd">
<network>
  <attributes>
    <attribute name="coordinateReferenceSystem" class="java.lang.String">EPSG:3006</attribute>
  </attributes>
  <nodes>
    <node id="123" x="674032.11" y="6580123.44"/>
    ...
  </nodes>
  <links capperiod="01:00:00" effectivecellsize="7.5" effectivelanelength="7.5">
    <link id="4211..." from="123" to="456" length="82.4" freespeed="13.9"
          capacity="3200" permlanes="2" modes="car"/>
    ...
  </links>
</network>
```

Every MATSim **link is directed** — which fits duckOSM exactly: our `edges` are already one row per
direction (`is_reverse`), so a two-way street is already two edges → two links. No `oneway` attribute
is needed (or exists) in the format.

## Mapping: duckOSM → MATSim

Read from a **built duckOSM routing db** (per-mode schema `driving.edges` / `driving.nodes`, like
[`to_sumo`](user_manual.md) / [`to_gis`](gis_export.md)) — *not* the GMNS db.

| MATSim | Source | Notes |
|---|---|---|
| `node/@id` | `nodes.node_id` | BIGINT → string |
| `node/@x`, `@y` | `ST_Transform(geom, crs)` | **reprojected to a metric CRS** (see below) |
| `link/@id` | `edges.edge_id` | the stable content-hash BIGINT → string; preserved verbatim |
| `link/@from`, `@to` | `edges.source`, `edges.target` | |
| `link/@length` | `edges.length_m` | true graph length in metres |
| `link/@freespeed` | `maxspeed_kmh / 3.6` | m/s; fallback `length_m / cost_s`, then class default |
| `link/@capacity` | `_CAPACITY[class] × permlanes` | veh/hour; reuse GMNS per-class table (motorway 2000…service 300) |
| `link/@permlanes` | `edges.lanes` | float ≥ 1 (per-direction; our class-default fill) |
| `link/@modes` | mode → MATSim mode | `driving→car`, `cycling→bike`, `walking→walk` |

### Coordinates & CRS (the one real design decision)

MATSim is a **metric** simulator: `freespeed × Δt` must equal a length in the *same units* as node
coordinates, so lon/lat degrees are wrong. duckOSM stores nodes in EPSG:4326, so we **reproject** node
coordinates with `ST_Transform` to a projected metric CRS and record it in the network `<attributes>`.

- Default **`EPSG:3006`** (SWEREF99 TM) — the Swedish national grid, right for the Stockholm/Tartu
  builds. Configurable via `--crs` (e.g. a UTM zone for other regions).
- `length` stays `length_m` (already true metres from the graph), independent of the node CRS — so
  lengths remain exact even though they're not recomputed from the projected coords.

### Defaults (same philosophy as GMNS, one source of truth)

- **freespeed**: prefer `maxspeed_kmh/3.6`; else `length_m/cost_s` (duckOSM's own routing free-flow
  time); else a per-class default (reuse a small speed table). Always > 0.
- **capacity**: `_CAPACITY[highway_class] × permlanes` (the `_CAPACITY` dict is imported from
  `gmns.py`, so GMNS and MATSim agree; values are per-lane veh/h).
- **permlanes**: `edges.lanes` (INTEGER per-direction fill, class default where OSM untagged — see
  [[lanes-mechanism]]); floored at 1.

## CLI & API

```bash
duckosm matsim sodermalm_pbf.duckdb                        # -> sodermalm_pbf_network.xml.gz (driving, EPSG:3006)
duckosm matsim sodermalm_pbf.duckdb -m driving --no-gzip -o net.xml
duckosm matsim tartu_pbf.duckdb --crs EPSG:32635           # UTM 35N for Tartu
```

```python
from duckosm import to_matsim
to_matsim("sodermalm_pbf.duckdb", "network.xml.gz", mode="driving", crs="EPSG:3006")
```

- `to_matsim(source, out_path, mode="driving", crs="EPSG:3006", gzip=True)` → returns counts
  `{"nodes": …, "links": …}`. Writes `.xml.gz` when `gzip` (MATSim convention) else plain `.xml`.
- Only nodes that are actually referenced by an exported link are written (drops isolated nodes).

## Scope

**v1 (this doc):** single-mode directed network.xml — nodes + links with length / freespeed /
capacity / permlanes / modes, reprojected coords, gzip. The high-value 90% that MATSim & BEAM need.

**Explicitly out of scope (follow-ons, noted not silently dropped):**
- **Multimodal single network** — one network with `modes="car,bike,walk"` per link, from the stitched
  `mm` schema (`mm.edges`). A clean follow-on once v1 lands.
- **`lanes.xml` + `signalSystems`** — MATSim's detailed intersection model; we already have the
  turn **movements** (from GMNS/meso) to feed it later.
- **GTFS transit, `plans`/`population`, `config.xml`** — demand & scenario, not network; out of scope.

## Fidelity

Topology, node positions, directedness, and length are **real**. `freespeed`/`capacity` are
class-derived wherever OSM leaves `maxspeed`/`lanes` untagged (same honest caveat as GMNS) — fine for
assignment/agent-based runs, and improvable later from the same enrichment paths.

## Tests & placement

- Module `src/duckosm/matsim.py` (`to_matsim`); CLI `matsim`; `to_matsim` exported from `__init__`.
- `tests/test_matsim.py`: XML parses (ElementTree) and is `network_v2`-shaped; node/link counts match
  the source edges/nodes; every link has `freespeed>0`, `capacity>0`, `permlanes≥1`, non-empty
  `modes`; coordinates are metric (large projected values, not degrees); referenced nodes all exist;
  `.xml.gz` round-trips. Reuse the tiny in-memory source fixture pattern from `test_gmns_map.py`.
- Docs: this file + `docs/README.md` index + a README command entry.
