# Draw a map

```bash
pip install "duckosm[viz]"
duckosm viz monaco.duckdb        # one map per mode, in reports/
```

Each map shows the roads coloured by class, with a legend, a base-map switcher, a hover tooltip and
a 2D/3D button. Click a road to see its `edge_id`. Tunnels are drawn under the roads above them,
bridges on top.

Try it: this is Monaco's driving map, as `duckosm viz` wrote it.

<iframe src="../../maps/monaco_driving.html" title="Monaco's driving network, drawn by duckosm viz"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

| Option | Does |
|---|---|
| `-m driving` | one mode only (default: every mode) |
| `--no-arrows` | no direction arrows (on by default, on one-way roads; zoom in to see them) |
| `--basemap positron` | first base map: `voyager` (default), `positron`, `dark_matter`, `osm`, `satellite`, `blank` |
| `--no-boundary` | hide the dashed outline of the clip area |
| `--out-dir maps` | output folder (default `reports`) |

**Other palettes, or colour by a column** (needs a clone of the repo):

```bash
python scripts/roadstyle_map.py --db monaco.duckdb --palette mono           # highsat | carto | mono
python scripts/roadstyle_map.py --db monaco.duckdb --color-by maxspeed_kmh
```
