# Draw a map

Draw the network of a database as HTML pages, with no server.

```bash
duckosm viz monaco.duckdb        # writes reports/monaco_map.html and reports/monaco_<mode>_network.html
```

The map is drawn by roadstyle, over the [base-map layers](../reference/features.md) of the database. Roads have a color
for each class, and tunnels and bridges follow the [drawing order](drawing-order.md). Private roads and bus lanes are drawn,
but they are not in the network ([why](../concepts/networks.md#access-private-and-forbidden-roads)). Click a road to see
its `edge_id`.

<iframe src="../../maps/monaco_map.html" title="Monaco, every mode, drawn by duckosm viz"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

| Option | Does | Default |
|---|---|---|
| `-m driving` | only the page of this mode; repeat it for more modes | every mode, and `<name>_map.html` |
| `--basemap voyager` | the first background: `voyager`, `positron`, `osm`, `satellite`, ... | the layers of the database |
| `--no-arrows` | no direction arrows on one-way roads | arrows on |
| `--no-boundary` | no outline of the area | outline on |
| `--google-3d` / `--no-google-3d` | a Google 3D window linked to the map; needs `GOOGLE_MAPS_KEY` | on when the key is set |
| `--out-dir maps` | the output folder | `reports` |

A page holds the whole network. Use it for a city. For a large area, use [duckOSM Studio](studio.md), which draws the map
from tiles.

To use a different palette, or a color for each value of a column, use the script of the repository (it needs
`pip install geopandas roadstyle`):

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```
