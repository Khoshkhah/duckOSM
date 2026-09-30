# Cut roads where any OSM way touches them

**Status:** implemented 2026-10-01 (rule set by Kaveh the same day).

## Problem

Roads were cut only where **roads** meet (and at way ends). A footway, path or cycleway sharing a
node with a road mid-way cut only itself, so walking and cycling had no connection there. Monaco:
616 such points, 361 of them crosswalks (e.g. the crosswalk on Avenue de Grande-Bretagne, edges
7304806875215893585 / 9024724288153699269, node 1079750854). The reason for the old rule: keep a
road's pieces, and so its `edge_id`, the same in every mode.

## Rule (Kaveh)

A road is cut at every node **another OSM way uses**: a road, a footway, a path or a cycleway; and
where a road ends. `main.global_junctions` is built from every highway way (not only roads), once,
so every mode cuts at the same points and a road keeps **the same `edge_id` in every mode**. Highway
ways that don't exist on the ground (`proposed`, `construction`, `abandoned`, `razed`, `disused`)
don't cut. Pieces are merged back only where no other way touches (as before).

## Effect (Monaco)

| | before | after |
|---|---|---|
| driving edges | 1,940 | 2,765 |
| walking edges | 8,948 | 10,706 |
| cycling edges | 8,448 | 10,228 |
| path–road contacts not connected | 616 | 0 |
| same road piece with different ids across modes | 0 | 0 |

**Ids change once** for the roads that now get cut (their pieces have new end nodes, so new
`hash(osm_id, source, target)`): projects keyed on driving ids re-match those edges once; ids are
stable across rebuilds again from then on.
