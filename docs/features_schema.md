# `features.*` — the base-map schema (Shortbread)

> Enable with `options.build_features: true` (off by default).

## Goal — one db, two consumers

duckOSM is the **single OSM-extraction layer**. One build produces, in **one `.duckdb`**:

- `raw.*` — parsed OSM (ST_READOSM)
- per-mode routing graphs (`driving`/`walking`/`cycling`) + `mm.*` (multimodal)
- **`features.*`** — every base-map theme duckmap needs

**duckmap no longer ingests OSM or builds its own db.** It opens the duckOSM `.duckdb` and renders
from `features.*` — pure styling. (Requested: "duckmap must use the duckdb from duckOSM, not create
another one.")

## The 3-tier schema

```
raw       raw.nodes / raw.ways / raw.relations           (ST_READOSM — untyped OSM + tags)
  │  ensure_foundation()  — build geometry from node refs
geom      geom.way_pt / way_line / rel_area              (TRANSIENT scaffolding, dropped after)
  │  the thematic layers filter + classify
features  features.streets / water_polygons / land / ...  (uniform contract, one table per theme)
```

`geom.*` is **dropped** once `features.*` are built (`drop_foundation`), so each geometry ends up in
exactly one place — no lasting duplication.

## Uniform column contract (every `features.*` table)

| column | type | meaning |
|---|---|---|
| `osm_id` | BIGINT | OSM element id |
| `osm_type` | VARCHAR | `'node'` / `'way'` / `'relation'` (id unique only within a type) |
| `kind` | VARCHAR | the **Shortbread** class value — styling/filter key |
| `name` | VARCHAR | `name` tag |
| `tags` | MAP(VARCHAR,VARCHAR) | full raw tags |
| `geom` | GEOMETRY | shape, EPSG:4326 lon/lat |

(`kind` is Shortbread's term for the class; the geometry-type point/line/area is implicit per table.)

## Layers — OSM → Shortbread mapping (implemented)

| `features.<layer>` | geom | from OSM | Shortbread `kind` |
|---|---|---|---|
| `streets` | line | `highway=*` **and** `railway∈{rail,tram,subway,light_rail,narrow_gauge,funicular,monorail}` | the highway or railway value (roads+rail **merged**, per Shortbread) |
| `water_polygons` | area | `natural=water/glacier`, `waterway=riverbank`, `landuse=reservoir/basin`, `water=*` | water/river/reservoir/basin/dock/glacier |
| `water_lines` | line | `waterway=river/stream/canal/ditch/drain` | canal/river/stream/ditch (drain→ditch) |
| `land` | area | `landuse=*`, `natural=wood/scrub/…`, `leisure=park/garden/…` | forest/grass/residential/… (wood→forest) |
| `sites` | area | `amenity=parking/bicycle_parking/school/university/hospital/prison`, `leisure=sports_centre`, `landuse=construction`, `military=danger_area` | parking/university/construction/… |
| `buildings` | area | `building=*` (≠no) | (building value; Shortbread has no kind) |
| `public_transport` | point | `highway=bus_stop`, `railway=station/halt/tram_stop`, `amenity=bus_station/ferry_terminal`, `aeroway=aerodrome/helipad` | bus_stop/station/halt/tram_stop/… |
| `pois` | point | `amenity/shop/tourism/office/leisure/man_made` (minus transit/parking) | the tag value |
| `place_labels` | point | `place=city/town/village/…` | the place value |
| `boundaries` | line | `boundary=administrative`, `admin_level∈{2,4}` | admin_level — **disabled stub** (needs relation-line build) |

**Verified on Tartu:** streets 13,366 (roads+rail), buildings 22,494, land 3,739, pois 2,970,
public_transport 373, sites 884, water_polygons 113, water_lines 289, place_labels 23.

### Not carried over (out of Shortbread scope)
duckmap's `barriers` and `power` layers have no Shortbread equivalent — they were disabled stubs in
duckmap and are omitted here. `bridges`, `street_polygons`, `ferries`, `aerialways`, `addresses`,
`street_labels_points`, and the water `dam/pier` sublayers are Shortbread layers not yet built —
register them in `layers.py` when needed (each is a predicate + `kind_sql`).

## How to build

- `options.build_features: true` in the config → `_build_features()` runs after the mode graphs
  (needs `raw.*`, still present pre-cleanup).
- Or standalone on an existing db that still has `raw.*`:
  ```python
  from duckosm.features import FeaturesBuilder
  FeaturesBuilder(con).run()          # con = writable duckdb with raw.*
  ```

## Phase B — duckmap becomes a pure renderer (pending)

- Delete duckmap's extraction (`geometry/`, `layers/` *building*, the `raw`/`geom`/`basemap` build in
  `pipeline.py`).
- Point duckmap at a duckOSM `.duckdb`; read `features.*` (rename its styler's `basemap.`→`features.`
  and `class`→`kind`). Keep only palettes/renderers.
- Multimodal viz: the transfer POIs are `features.sites` (parking) + `features.public_transport`;
  overlay `mm.transfers` as connector lines and colour `mm.edges` by mode.

## Code

`src/duckosm/features/`: `geometry.py` (foundation + `build_{area,line,point}_layer`),
`layers.py` (Shortbread `Layer` classes + `LAYERS`), `builder.py` (`FeaturesBuilder`). Ported from
duckmap's verified engine — duckOSM's `raw` schema is column-identical, so the move is low-risk.
