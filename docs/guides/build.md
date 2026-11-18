# Build a network

`duckosm build` makes the routable networks of an area in one DuckDB file. The steps: [How a build works](../concepts/build.md).

```bash
duckosm build --pbf monaco-latest.osm.pbf -b monaco.geojson -m driving -m walking -m cycling
```

The output `monaco.duckdb` has one schema for each mode (edges, nodes, the graph of legal turns) and the
[base-map layers](../reference/features.md). With a boundary, the build first cuts the PBF to it with osmium and keeps the
cut in `./pbf/`. The next build of the same area uses that cut again, until the boundary or the PBF changes.
Without osmium, the build uses the whole PBF.

| Option | Does | Default |
|---|---|---|
| `-p`, `--pbf FILE` | the input `.osm.pbf` | |
| `-b`, `--boundary FILE` | the area ([make one](prepare-area.md#make-the-boundary-file)) | none: the whole PBF |
| `-m`, `--modes MODE` | `driving`, `walking` or `cycling`; repeat for each mode | `driving` |
| `-o`, `--output FILE` | the output file | `<boundary name>.duckdb`, else `<pbf name>.duckdb` |
| `-c`, `--config FILE` | a [config file](#the-config-file) | |
| `--fixes FILE` | a [fixes file](edit-osm-data.md#the-fixes-file) | none |

All options: [Command line](../reference/cli.md#build).

## The config file

A config file keeps the settings of a build. `duckosm init-config` writes the template with all keys and their defaults.

```bash
duckosm init-config monaco.yaml
duckosm build -c monaco.yaml
```

| Key | Does | Default |
|---|---|---|
| `name` | the name of the output, `<name>.duckdb` | `default` |
| `source.pbf_path` | the input `.osm.pbf` | |
| `boundary.path` | the area as a GeoJSON file (or `boundary.place`, `bbox`, `h3_cell`) | none: the whole PBF |
| `modes` | the networks to build | `[driving]` |
| `osm_overrides` | a [fixes file](edit-osm-data.md#the-fixes-file) | none |
| `options.build_features` | make the base-map layers | `true` |
| `options.levels` | make the [drawing order](drawing-order.md) of the roads; set `false` for a large area | `true` |
| `options.memory_limit` | the memory limit of DuckDB, for example `"16GB"` | the DuckDB default |

```yaml
name: monaco
source:
  pbf_path: monaco-latest.osm.pbf
boundary:
  path: monaco.geojson
modes: [driving, walking, cycling]
osm_overrides: osm_fixes.yaml
```

A key that is not in the file has its default. A relative path starts from the folder where you run `duckosm`. An option
that you type overrides the file, for example `duckosm build -c monaco.yaml -m driving`.
Every key: [Configuration](../reference/configuration.md).

For a country, build the region one time and [cut the areas from it](prepare-area.md#several-areas-from-one-region).
To build a large area tile by tile, give the same config file to `duckosm tiles --config monaco.yaml`.
