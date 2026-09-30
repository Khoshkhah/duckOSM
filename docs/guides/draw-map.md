# Draw a map

```bash
pip install "duckosm[viz]"
duckosm viz monaco.duckdb        # -> reports/monaco_<mode>_network.html, one map per mode
```

Each map shows the roads coloured by class, with a legend, a base-map switcher, a hover tooltip and
a 2D/3D button. Click a road to copy its `edge_id`. Tunnels are drawn under the roads above them,
bridges on top.

| Option | Does |
|---|---|
| `-m driving` | one mode only (default: every mode) |
| `--arrows` | direction arrows on one-way roads (zoom in to see them) |
| `--basemap positron` | first base map: `voyager` (default), `positron`, `dark_matter`, `osm`, `satellite`, `blank` |
| `--no-boundary` | hide the dashed outline of the clip area |
| `--out-dir maps` | output folder (default `reports`) |

**Other palettes, or colour by a column** (needs a clone of the repo):

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --palette mono           # highsat | carto | mono
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```
