# Build a network

```bash
duckosm build --pbf monaco-latest.osm.pbf -b monaco.geojson -m driving -m walking -m cycling
```

This writes `monaco.duckdb`: one schema per mode (`driving`, `walking`, `cycling`), each with its
edges, nodes, the graph of legal turns and, for driving, the turn restrictions. With a boundary
(`-b`), the build first cuts the PBF to it (a copy is kept in `./pbf/` for the next build) and removes
small pieces of network that don't connect to the rest. What each step does:
[How a build works](../concepts/build.md).

## Options

| Option | Does | Default |
|---|---|---|
| `-p`, `--pbf` | input `.osm.pbf` | |
| `-b`, `--boundary` | GeoJSON area to build ([make one](prepare-area.md)) | none: the whole PBF |
| `-m`, `--modes` | `driving`, `walking` or `cycling`; repeat for several | `driving` |
| `-o`, `--output` | output file | `<boundary name>.duckdb`, else `<pbf name>.duckdb` |
| `-c`, `--config` | a config file (below) | |
| `--source-db` | cut from a built db instead of a PBF ([how](prepare-area.md#several-areas-from-one-region)) | |
| `--graph` / `--no-graph` | build the graph of legal turns | on |
| `--h3-index` / `--no-h3-index` | add H3 cell ids to nodes and edges | on |
| `--h3-resolution` | H3 resolution, 0–15 | `8` |
| `--fixes` | a rules file of [fixes for OSM errors](fix-osm-errors.md) | none |
| `--log-file` | also write the log to a file | console only |

## With a config file

For repeatable builds, keep the settings in a file. `duckosm init-config` writes a fully commented
template:

```bash
duckosm init-config monaco.yaml          # edit it, then:
duckosm build -c monaco.yaml
```

```yaml
name: monaco                           # output: <output_path>/monaco.duckdb
output_path: .
source:
  pbf_path: monaco-latest.osm.pbf
boundary:
  path: monaco.geojson
modes: [driving, walking, cycling]
validation:
  enabled: true                        # check the result (see "Check a build")
```

A config file also reaches settings the command line doesn't have: [validation](check-build.md),
[fixes for OSM errors](fix-osm-errors.md), memory limits, and more. Every field:
[Configuration](../reference/configuration.md). In a clone of the repo, `config/sample_monaco.yaml` is a ready
example.

## Large areas

A whole country takes time and memory (Sweden: 835 MB of PBF). The build processes the network in
batches sized to fit; if it still runs out of memory, set `options.memory_limit` (e.g. `"16GB"`) and
`options.threads` in the config. Build a big region once and [cut areas from it](prepare-area.md#several-areas-from-one-region)
instead of rebuilding each one.
