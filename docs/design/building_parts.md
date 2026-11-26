# Building parts: links, heights in metres and walls

Status: done (2026-11-26; approved by Kaveh). Asked by noise-map (a road-noise model of Norrmalm).

## Problem

PR 488 added `features.building_parts`: each OSM `building:part` with its tags. A user that needs a building in 3D
(noise-map: noise points on the walls, and the building heights for NoiseModelling) cannot use the table as it is:

| Gap | Now | Effect |
|---|---|---|
| 1. A part has no link to its building | no column | each user makes a spatial join |
| 2. No height in metres | only the tags: `height`, `building:levels`, `min_height`, `building:min_level` | each user has its own rule (noise-map: storeys × 3 m; roadstyle and Studio: × 3.2 m) |
| 3. A part with a base above the ground | `min_height` or `building:min_level` > 0 (14 parts in Norrmalm) | a user cannot tell a part on top of another part (a tower) from a part over a street (a passage) |
| 4. The walls are only on the building outlines | `main.building_faces` | the walls where a tall part rises above a lower part are in no table, and the height of an outline wall changes from part to part |
| 5. `overture-buildings` reads only whole buildings | Overture `building` type | the parts get no Overture values |

## Design

One Rust function, `duckosm_duckdb::building_parts::refresh(db)`, does points 1 to 4 in SQL. The build runs it after
the features (and after the walls, when `--building-faces` is on). `duckosm overture-buildings` runs it again at the
end, because it changes the tags. The Python `FeaturesBuilder` and `features.sql` do not change.

### 1. The link

New columns of `features.building_parts`: `building_id BIGINT` and `building_type VARCHAR` (`way` or `relation`).

1. An OSM relation `type=building` gives the link: the member with the role `outline` is the building, the members
   with the role `part` are its parts.
2. Else: the building whose outline holds the point on the surface of the part (`ST_PointOnSurface`). When more than
   one does, the smallest.
3. Else: NULL. The build prints the number of these parts.

### 2. Heights in metres

New columns of `features.buildings` and `features.building_parts`:

| Column | Value |
|---|---|
| `height_m DOUBLE` | the top: `height` (metres; `ft`, `feet` and `'` are converted), else `building:levels` × 3.2 m, else 6 m |
| `min_height_m DOUBLE` | the base: `min_height`, else `building:min_level` × 3.2 m, else 0 |
| `height_source VARCHAR` | `height`, `levels` or `default` (the source of `height_m`) |
| `has_parts BOOLEAN` | only in `features.buildings`: one or more parts link to it |

This is the rule of duckOSM's pages (`heightOf` in `web/fixes_editor/fixes_panel.js`, `building_info` in Studio).
`roof:levels` and `roof:height` are not added: the rule gives the height of the walls. A building with parts keeps its
own height. A 3D user draws the parts in place of the building (OSM Simple 3D Buildings).

### 3. The base of a part

New column of `features.building_parts`: `support VARCHAR`.

| Value | When |
|---|---|
| `ground` | `min_height_m` = 0 |
| `stacked` | other parts of the same building cover at least 90 % of its footprint, and their top is at least its base minus 0.5 m |
| `floating` | every other case (for example a passage over a street) |

A user can make a `stacked` part solid from the ground. It must keep the empty space under a `floating` part.

### 4. The walls of the parts

New table `main.building_part_faces`, made when `--building-faces` is on. One row for each segment of the outer ring
of each part:

| Column | Value |
|---|---|
| `part_id`, `part_type`, `building_id`, `building_type` | the part, and its building |
| `polygon`, `face_index` | the polygon of the part (for a multipolygon) and the place of the wall in its ring |
| `from_node`, `to_node`, `length_m`, `geometry` | as in `main.building_faces` (the nodes only for a part that is a way) |
| `z_bottom_m`, `z_top_m` | the base and the top of the part |
| `open_from_m`, `open_to_m` | the height range of the wall that is open to the air: from `z_bottom_m`, or from the top of the parts that close the wall from its base up (a part that touches the wall and starts at or under its base, then the parts that start on top of that one), to `z_top_m`. NULL when they close all of it (the wall is inside the building) |
| `position` | `outline`: the segment is on the outline of the building; `step`: inside the outline, where a part rises above another part |
| `type` | for an `outline` segment: the type of the wall of the building at that place (`front`, `back`, `side`, `party`, `court`); for a `step`: `step` |

Two parts touch a segment when their rings have the same two nodes (OSM parts share nodes). An `outline` segment takes
the type of the wall of `main.building_faces` of its building that is within 0.5 m of its middle point, in the same
direction.

Example: the four parts of way 440810380 (8, 11, 14 and 16 levels). The 16-level part has `step` walls from 44.8 m (the
top of the 14-level part) to 51.2 m, and `outline` walls from 0 to 51.2 m with the types of the hotel's walls.

Limit: two parts that touch with no shared nodes (the mapper drew two lines) give two walls that are both open. The
check of this case is a later step, if a user needs it.

### 5. Overture for the parts

`duckosm overture-buildings` also reads the Overture type `building_part`, and adds its values to
`features.building_parts` with the same rules (OSM wins; `--overwrite`). The summary gives the counts of the parts.

### 6. A build for noise-map

After the merge, a build from `main` has all of it:

```bash
cargo build --release -p duckosm-cli
rust/target/release/duckosm build --config CONFIG --features --building-faces --dem DEM
rust/target/release/duckosm overture-buildings DB
```

## Tests

- Monaco: the number of parts with a link, of each `support` value, and the rows of `main.building_part_faces`.
- A small database in SQL with three parts (two stacked, one floating) and the open ranges of their walls.
- Norrmalm (local): way 440810380 gives the walls of the example.

## Docs

`docs/reference/database.md` (the new columns and table), `docs/reference/features.md`, `docs/concepts/building-walls.md`
(the walls of the parts), `docs/reference/cli.md` (`overture-buildings`), `CHANGELOG.md`.
