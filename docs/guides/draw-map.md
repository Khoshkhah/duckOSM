# Draw a map

```bash
duckosm viz monaco.duckdb        # writes reports/monaco_map.html (every mode) and one page for each mode
```

`duckosm viz` draws the maps with roadstyle's engine, from the database itself. Under the roads are the
[base-map layers](../reference/features.md) of the database: water, land, buildings and the sea. The page needs no
background from the web, but you can select one.

| On the map | How it shows |
|---|---|
| Roads | a color for each road class |
| Tunnels and bridges | tunnels under the roads above them, bridges on top ([Store the drawing order](drawing-order.md)) |
| Private roads, bus lanes | drawn, but not in the network ([why](../concepts/networks.md#access-private-and-forbidden-roads)) |
| A click on a road | the panel shows the road and its `edge_id` |

`monaco_map.html` has every mode. Each `monaco_<mode>_network.html` has the edges and the private roads of one mode.
This is Monaco with every mode:

<iframe src="../../maps/monaco_map.html" title="Monaco, every mode, drawn by duckosm viz"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

| Option | Does | Default |
|---|---|---|
| `-m driving` | only the page of that mode; repeat it for more modes | every mode, and `<name>_map.html` |
| `--basemap voyager` | the first background: `voyager`, `positron`, `osm`, `satellite`, … | the database's own layers |
| `--no-arrows` | no direction arrows on one-way roads | arrows on |
| `--no-boundary` | no dashed outline of the area | outline on |
| `--google-3d` / `--no-google-3d` | a Google 3D window linked to the map; needs `GOOGLE_MAPS_KEY` | on when `GOOGLE_MAPS_KEY` is set |
| `--out-dir maps` | the output folder | `reports` |

A page holds the whole network. Use it for a city, not a country. For a large area, use
[duckOSM Studio](studio.md), which draws the map from tiles.

## Other palettes, or a color for each value of a column

This script is in the repository. It needs `pip install geopandas roadstyle`.

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --palette mono
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```

To set the order in which the build paints the roads at a crossing, see [Store the drawing order](drawing-order.md).
