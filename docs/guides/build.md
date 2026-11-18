# Build a network

```bash
duckosm build --pbf monaco-latest.osm.pbf -b monaco.geojson -m driving -m walking -m cycling
```

This writes `monaco.duckdb`: one schema per mode (`driving`, `walking`, `cycling`), each with its
edges, nodes, the graph of legal turns and, for driving, the turn restrictions; and the
[base-map layers](../reference/features.md) (`features`: water, land, buildings, POIs). With a
boundary (`-b`), the build first cuts the PBF to it (a copy is kept in `./pbf/` for the next build,
until the boundary or the PBF changes) and removes small pieces of network that don't connect to the
rest. What each step does:
[How a build works](../concepts/build.md).

## Options

| Option | Does | Default |
|---|---|---|
| `-p`, `--pbf` | input `.osm.pbf` | |
| `-b`, `--boundary` | GeoJSON area to build ([make one](prepare-area.md)) | none: the whole PBF |
| `-m`, `--modes` | `driving`, `walking` or `cycling`; repeat for several | `driving` |
| `-o`, `--output` | output file | `<boundary name>.duckdb`, else `<pbf name>.duckdb` |
| `-c`, `--config` | a [config file](#the-config-file) | |
| `--source-db` | cut from a built db instead of a PBF ([how](prepare-area.md#several-areas-from-one-region)) | |
| `--graph` / `--no-graph` | build the graph of legal turns | on |
| `--h3-index` / `--no-h3-index` | add H3 cell ids to nodes and edges | on |
| `--h3-resolution` | H3 resolution, 0–15 | `8` |
| `--no-features` | skip the [base-map layers](../reference/features.md) (water, land, buildings, POIs) | built |
| `--fixes` | a rules file of [fixes for OSM errors](fix-osm-errors.md) | none |
| `--log-file` | also write the log to a file | console only |

## The config file

A config file keeps the settings of a build, so that you can build the same area again with one
command. Write the commented template, change some keys, and build:

```bash
duckosm init-config monaco.yaml          # writes the template with all keys and their defaults
duckosm build -c monaco.yaml
```

### The keys that you change most

| Key | Does | Example | Default |
|---|---|---|---|
| `name` | The name of the output file, `<name>.duckdb` | `monaco` | `default` |
| `source.pbf_path` | The input `.osm.pbf` file | `monaco-latest.osm.pbf` | |
| `boundary.path` or `boundary.place` | The area: a GeoJSON file, or a place name (the PBF's borders, then Nominatim) | `monaco.geojson` or `"Monaco"` | none: the whole PBF |
| `modes` | The networks to build | `[driving, walking, cycling]` | `[driving]` |
| `osm_overrides` | A rules file of [fixes for OSM errors](fix-osm-errors.md) | `osm_fixes.yaml` | none |
| `options.build_features` | Build the [base-map layers](../reference/features.md) (water, land, buildings, POIs) | `false` | `true` |
| `options.levels` | Make the [drawing order](drawing-order.md) of the roads as the last step. Set `false` for a large area. | `false` | `true` |
| `options.h3_resolution` | The H3 resolution of the cell columns, 0 to 15 | `9` | `8` |

The config file has no key for tiles. To build a large area by tiles, give the same file to
`duckosm tiles --config monaco.yaml`.

### A complete small file

```yaml
name: monaco
output_path: .                         # a folder, or a full *.duckdb path
source:
  pbf_path: monaco-latest.osm.pbf
boundary:
  path: monaco.geojson
modes: [driving, walking, cycling]
osm_overrides: osm_fixes.yaml
options:
  build_features: true
  levels: true
  h3_resolution: 8
```

A key that is not in the file has its default. Relative paths start from the folder where you run
`duckosm`.

### The same config from Python

```python
import duckosm

db = duckosm.build(config="monaco.yaml")                       # as duckosm build -c monaco.yaml
db = duckosm.build(config="monaco.yaml", modes=["driving"])    # an option that you give overrides the file
```

On the command line, an option that you type also overrides the file:
`duckosm build -c monaco.yaml -m driving`.

Every key: [Configuration](../reference/configuration.md). All Python functions:
[Python SDK](../reference/python.md). In a clone of the repository, `config/sample_monaco.yaml` is a
ready example.

## Large areas

A whole country takes time and memory (Sweden: 835 MB of PBF). The build processes the network in
batches sized to fit; if it still runs out of memory, set `options.memory_limit` (e.g. `"16GB"`) and
`options.threads` in the config. Build a big region once and [cut areas from it](prepare-area.md#several-areas-from-one-region)
instead of rebuilding each one.
