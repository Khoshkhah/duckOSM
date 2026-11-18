# How each output is checked

Each output is compared with a reference: another tool, a schema or a specification.

| Output | Reference | Check | Test |
|---|---|---|---|
| OSM features table (`quack`) | QuackOSM | the counts of the rows equal QuackOSM's; each row: `cargo run --example quack_diff` (duckosm-duckdb) | `rust/duckosm-core/tests/quack_monaco.rs` |
| GDAL's layers (`quack --gdal`) | GDAL's OSM driver | each row equals GDAL's (Monaco, Kentucky) | `rust/duckosm-build/tests/quack_gdal_monaco.rs` |
| SUMO | SUMO's `netconvert` | `netconvert` reads the network | `tests/test_sumo.py` |
| MATSim | MATSim's DTD and XSD schemas | the file is valid | `tests/test_matsim.py`, `tests/test_matsim_lanes.py` |
| GMNS | the GMNS specification | the tables follow the specification | `tests/test_gmns_spec.py` |
| GeoPackage, shapefile | GDAL | `duckosm gis-debug` reads the file back and compares each `edge_id` | `tests/test_gis.py`, `tests/test_gis_debug.py` |
| Browser build | the native build | the tables are equal | `cargo test -p duckosm-wasm` in [duckOSM Studio](https://github.com/Khoshkhah/duckosm-studio) |

More: [duckOSM and QuackOSM](quackosm.md), [duckOSM and GDAL](gdal.md), [Browser and native builds](browser-native.md).
