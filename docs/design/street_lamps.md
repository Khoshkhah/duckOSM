# Design: street lamps as a feature layer

Status: proposal. No code is written. It waits for Kaveh's sign-off.

## Problem

A map made from a duckOSM database shows crossings and traffic signals, but no street lights.
`features.traffic` holds only `highway=traffic_signals` and `highway=crossing`
(`src/duckosm/features/layers.py`, class `Traffic`). Nothing reads `highway=street_lamp`.

## Data

OSM maps a street light as a node with `highway=street_lamp`. The count is low, because OSM
coverage of street lights is low:

| Area (local PBF file) | `street_lamp` | `crossing` | `traffic_signals` |
|---|---|---|---|
| Monaco | 28 | 553 | 22 |
| Södermalm | 190 | 596 | 219 |

## Change in duckOSM

Add `street_lamp` to the `Traffic` layer. The layer already makes point features from tagged
nodes, so the change is small:

- `where_sql`: `highway IN ('traffic_signals','crossing','street_lamp')`.
- `kind_sql`: unchanged (`highway`), so `kind = 'street_lamp'`.
- No `bearing` for a lamp (only crossings have one).
- `docs/reference/database.md`: `features.traffic` has the kinds `traffic_signals`, `crossing`
  and `street_lamp`.
- A test on the Monaco sample: `features.traffic` has 28 rows with `kind = 'street_lamp'`.

A database that was built before this change has no lamps. Build it again.

## Change in mapstyle

- `styles/layers.yaml`: a new row, `street_lamps`, with `table: traffic`,
  `where: "kind = 'street_lamp'"`, `kind: point`, `ticked: false`, as the rows `traffic_signals`
  and `crossings` have.
- An icon for the lamp (a small SVG, as `ms-icon-bus`), and a minimum zoom of 17.
- The GeoLibre plugin needs no change: a new layer group becomes a new row in the `features`
  folder of the Layers panel.

A mapstyle with the new row works on a database that has no lamps: the layer is empty.
So the order of the releases is free.

## Decisions for Kaveh

1. **Table.** Add the lamps to `features.traffic` (recommended: one line, and the table
   already means "road furniture"), or make a new table `features.street_lamps`.
2. **Default.** Start the layer hidden (`ticked: false`, as the other furniture), or shown.
3. **Icon.** A simple lamp pictogram (recommended), or a plain dot.

## Not in scope

Other street furniture (benches, bollards, bins) has the same pattern. Add it when the lamp
layer works.
