# The sea (`features.ocean`)

**Status:** implemented 2026-09-30 (approved by Kaveh: "go ahead"); mapstyle `docs/PLAN.md`, step 3.

## Problem

duckOSM builds no sea. OSM has no sea polygons: the sea is only implied by the `natural=coastline`
lines (land on their left), stitched worldwide into one closed coast. `features.water_polygons`
holds lakes, rivers, basins and pools, never the sea. So on mapstyle's `blank` base map a coastal
area's sea is land-coloured (Monaco: the whole south-east of the map).

## Options

| Option | How | Tried on | Verdict |
|---|---|---|---|
| **A. Overture Maps `base/water`, subtype `ocean`** | read straight from Overture's public GeoParquet on S3 with DuckDB (`httpfs`), only the row groups whose `bbox` overlaps the area; clip | Monaco: 1 polygon, 733 vertices, 1.8 s. Stockholm (17.5–19.0 E, 59.15–59.5 N): 3 polygons, 328k vertices, 1.3 s | **recommended** |
| B. osmdata.openstreetmap.de water polygons (what openstreetmap-carto uses) | download `water-polygons-split-4326.zip` once (906 MB, updated daily), cache, clip each build | not tried | the standard, but a 906 MB download per machine |
| C. Build it from the extract's own coastline lines | polygonize the clipped coastlines with the boundary, keep faces right of the coast | Monaco right; Stockholm county wrong (a session prototype) | fragile: open ends at the clip edge, one gap floods the land |

**Why A.** It is the same thing as B (Overture's ocean is built from OSM's coastline, ODbL), but
DuckDB reads only the part it needs, as duckOSM's elevation already does with remote rasters: no
big download, no cache to manage, a few seconds per build. Checked point by point:

| Point | Expected | A gives |
|---|---|---|
| off Monaco-Ville | sea | sea |
| Monaco-Ville, Casino | land | land |
| Mareterra (Monaco's new land, 2024) | land | land |
| Port Hercule | sea | sea |
| Baltic, outer archipelago | sea | sea |
| Strömmen by the Royal Palace, Saltsjön off Fåfängan | sea | sea |
| Skeppsholmen, Sergels torg | land | land |
| Riddarfjärden (Lake Mälaren) | not sea | a lake (`water`, not `ocean`) |

**Its costs:**
- **Network at build time.** Offline, the step warns and is skipped; the build doesn't fail (as
  with elevation).
- **Freshness.** Overture releases monthly (B: daily). A coastline change shows up within about a
  month.
- **The release path changes monthly.** The latest release is found by listing
  `s3://overturemaps-us-west-2/release/` (0.7 s), and recorded in the db's metadata.

## Design

- **Table:** `features.ocean` (geometry only), Shortbread's own layer name for the sea (Shortbread
  takes it from B; we take the same thing from A).
- **Built** when `build_features` is true, after the other feature layers.
- **Extent:** the boundary's bounding box (without a boundary, the extract's nodes') grown by 10 % (at least 500 m), clipped with
  `ST_Intersection`, so the sea also fills the view a little beyond the area's edge, where the
  other feature layers (complete ways from the smart clip) reach too.
- **Inland areas** (Tartu): no `ocean` rows, an empty table, no error. (Södermalm is coastal:
  Saltsjön is sea, Riddarfjärden a lake in `water_polygons`.)
- **Config:** `sea: overture` (default) | `false`. (B or a local file can be added later as
  another value; not built now.)
- **Attribution** on the map: "© OpenStreetMap contributors" already covers it (ODbL); the
  metadata records "Overture Maps, release …".
- **Size:** Monaco is negligible. Stockholm county's box has 328k vertices (a few MB inline): for
  large areas mapstyle simplifies it (or uses tiles, mapstyle PLAN step 2).

## mapstyle (after this)

- `styles/layers.yaml`: `ocean` first, under everything, in openstreetmap-carto's water colour
  (`#aad3df`, as `water_polygons`).
- The `blank` base map stays the land colour; the sea is drawn on it.

## Checks

- Monaco build: `features.ocean` covers the point off Monaco-Ville and not the Casino.
- Tartu build: `features.ocean` is empty, the build doesn't fail.
- Offline (no network, mocked): a warning, no `ocean` rows, the build doesn't fail.
- mapstyle: Monaco's page shows the sea (a screenshot, before / after).
