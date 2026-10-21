# Design: the polygons that the features build misses

**Status:** approved 2026-10-21, implemented on the branch `missing-polygons` (not pushed). Result for Monaco: `paved` has 83 areas (33 piers,
19 pedestrian areas, 12 helipads, 9 squares, …), `water_polygons` 88 (the 61 pools), and `duckosm features-check` shows no group without a reason.

## Problem

The map of Monaco has empty places that OSM does not leave empty. `FeaturesBuilder` keeps only the polygons that a layer
asks for (`land`, `water_polygons`, `sites`, `buildings`). Some polygon kinds that OSM has are in no layer. Two checks on the
Monaco database (2026-10-21) show it:

1. The polygon layers cover 78% of the boundary (2.39 km²). The other 22% is road surface, pavement and ground that nobody
   mapped as an area. This part is not a fault of the build.
2. **331 closed ways and 9 area relations of the raw data are in no `features.*` table.** Most are small. A few are the open
   places in the middle of the map.

| Kind in OSM (Monaco) | Closed ways | Relations | Area | Today |
|---|---|---|---|---|
| `place=square` (Place du Casino, Place Princesse Gabriella, Belvédère Gérard Brianti, …) | 6 | 3 | 3,628 m² (ways only) | no layer |
| `area:highway=*` (pedestrian, traffic island) | 3 | 0 | 2,031 m² (one way) | no layer |
| `highway=pedestrian / footway / service / … + area=yes` | 19 | 0 | | only the outline, as a line in `streets` |
| `man_made=pier / breakwater / groyne` | 34 | 1 | 6,962 m² | no table |
| `aeroway=helipad / heliport` | 13 | 0 | 10,677 m² | no table (a node is a point in `public_transport`) |
| `leisure=swimming_pool` | 61 | 0 | 7,920 m² | no table |
| `leisure=beach_resort` | 3 | 0 | 1,660 m² | no table |
| `natural=shrubbery` | 0 | 1 | | no layer |
| `tourism=zoo` | 1 | 0 | 5,547 m² | no table |
| `building:part=*` (a tower on a base, a wing) | 61 | 1 | 41,638 m² | no layer |

Not ground, so not in this note: `place=neighbourhood` (two zones, 117,025 m²) and the administrative boundaries.
Small furniture (power generators, motorcycle parking, fountains, outdoor seating) is also left out.

The comparison used the counts and the ids, not the shapes. The 83 ways with only unusual tags (156,246 m²) are probably the
rings of relations that the build holds. This must be checked again after the change.

## Change in duckOSM

All changes are in `src/duckosm/features/layers.py`, in the same declarative style. A closed way and a multipolygon relation are
both built by `build_area_layer`, so a rule is only a `where_sql` and a `kind_sql`.

1. **A new layer `paved`** (area): ground that people walk or stand on, and that the sea does not cover.
   - `place=square`, `area:highway=*`, `highway` in (`pedestrian`, `footway`, `service`, `living_street`, `steps`, `path`,
     `cycleway`) with `area=yes` (or a multipolygon relation), `man_made` in (`pier`, `breakwater`, `groyne`, `quay`), `aeroway` in
     (`helipad`, `heliport`), `amenity=marketplace`. The first run of the report on Monaco showed the quays, the market and the
     pedestrian relations; they are in.
   - `kind`: `square`, `pedestrian`, `footway`, `service`, `traffic_island`, `pier`, `breakwater`, `helipad`, … (the value).
   - It sits after `water_lines` in the drawing order, so a pier is over the water and under the buildings.
2. **`water_polygons`**: add `leisure=swimming_pool` (`kind` = `swimming_pool`).
3. **`land`**: add `leisure=beach_resort` and `natural=shrubbery` (`kind` = the value).
4. **`sites`**: add `tourism=zoo`.
5. **A report, `duckosm features-check DB`.** It lists the closed ways and the area relations of `raw.*` that are in no
   `features.*` table, grouped by tag, with counts and areas. It is the query of this note. It makes every future miss visible,
   and it is the test of this change: after the change, the Monaco report must show only the groups that the note leaves out on
   purpose (neighbourhoods, boundaries, furniture, building parts, relation rings).
6. **Phase 2, later:** a layer `building_parts` (area) with `height`, `min_height`, `building:levels` and the roof tags, so that
   mapstyle can draw a tower on a base in 3D with two heights. It needs its own design after the 3D buildings have been used.

The tables of old databases do not change. A database with `raw.*` gets the new layers with `FeaturesBuilder(con).run()`.
A clipped database (no `raw.*`) does not have them, and mapstyle skips a layer whose table is missing (as it does today).

Tests: the Monaco build has a `features.paved` with at least the 6 + 3 squares and the piers; `features.water_polygons` has the 61
pools; `duckosm features-check` runs and its groups are the expected ones. Docs: `docs/reference/features.md` (the table of layers),
`docs/reference/cli.md`, `CHANGELOG.md`. This is duckOSM 0.6.0 (a new table).

## Change in mapstyle

mapstyle reads the tables that `layers.yaml` names, so every new table is a row there. No change in roadstyle.

- `layers.yaml`: a layer `paved` (`table: paved`, `kind: polygon`, ticked) between `waterways` and `buildings`. The pools are
  already inside the `water` layer, and the beach resorts and the shrubbery inside `landcover`; the zoo is a `site` and comes
  with `institutional`.
- `osm_carto.yaml` (and the two other themes): colours for `paved` by `kind` (OSM-Carto: squares and pedestrian areas
  `#ededed` with a `#bbbbbb` outline, piers `#f5f5f0`, helipads `#e0e0e0`), and the new landcover classes (`beach_resort`,
  `shrubbery`).
- The export (`mapstyle.export`) and the GeoLibre plugin get the new layer without a change: it is one more row in the folder
  Background.
- This is mapstyle 0.8.0. The plugin is built again from the released packages.

## Decisions for Kaveh

1. One layer `paved` for squares, pedestrian areas, piers and helipads, as above? The other way is one layer for each kind (more rows
   in the panel). Recommendation: one layer.
2. The name `paved`? Other names: `ground`, `plazas`.
3. Is the report `duckosm features-check` wanted? Recommendation: yes. It is the way to find the next miss.
4. `building_parts`: later (phase 2), as above?
