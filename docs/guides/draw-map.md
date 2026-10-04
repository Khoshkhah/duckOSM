# Draw a map

```bash
pip install "duckosm[viz]"
duckosm viz monaco.duckdb        # in reports/: monaco_map.html (every mode) + one page per mode
```

The maps are drawn by [mapstyle](https://khoshkhah.github.io/mapstyle/), duckOSM's map library, from
the database itself: water, land, buildings and the sea ([base-map layers](../reference/features.md))
under the roads, so the page needs no background from the web (one can still be picked). The roads
are coloured by class, with a hover tooltip and a 2D/3D button; click a road to copy its `edge_id`.
Tunnels are drawn under the roads above them, bridges on top. Private roads are grey and bus lanes a
muted blue: you see them, but they are not in the network
([why](../concepts/networks.md#access-private-and-forbidden-roads)). In the Roads box, the road
classes and the Bridges, Tunnels, Private roads and Bus lanes rows hide or show those roads.

`monaco_map.html` has every mode; each `monaco_<mode>_network.html` brings one mode to the front.
Try it: this is Monaco with every mode, as `duckosm viz` wrote it.

<iframe src="../../maps/monaco_map.html" title="Monaco, every mode, drawn by duckosm viz"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

| Option | Does |
|---|---|
| `-m driving` | only that mode's page (repeat for several); without `-m`: every mode's page plus `<name>_map.html` |
| `--basemap voyager` | a background from the web under the map: `voyager`, `positron`, `osm`, `satellite` (default: none, the database's own layers) |
| `--no-arrows` | no direction arrows (on by default, on one-way roads; zoom in to see them) |
| `--no-boundary` | hide the dashed outline of the area |
| `--out-dir maps` | output folder (default `reports`) |

A page holds the whole network, so keep it to a city. For other looks, or to build your own
dashboard, use mapstyle directly (`pip install mapstyle`; its themes, path styles and dashboard
page are in its [docs](https://khoshkhah.github.io/mapstyle/)).

**Other palettes, or colour by a column** (needs a clone of the repo):

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --palette mono           # highsat | carto | mono
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```

The order in which crossing and meeting roads are painted can be stored in the file: [Store the drawing order](drawing-order.md).
