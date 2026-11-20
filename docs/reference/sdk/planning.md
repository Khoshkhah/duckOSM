# duckosm.planning

Transport planning: GMNS networks with lanes and movements.

| Call | Returns | Today |
|---|---|---|
| `to_gmns(net, out_path, mode=None, modes=None, to_csv=None, lane_geometry=True, combined=False)` | the GMNS tables in a DuckDB file; `to_csv` also writes the CSV files. A `ModeView` or `mode=` gives one mode, else `modes` (default all). | yes |
| `check_gmns(path, modes=None)` | the checks of the GMNS tables: `[(schema, check, bad, total), ...]` | yes |
| `to_meso(gmns_db, ...)`, `to_micro(gmns_db, ...)` | the meso and micro networks | yes |
| `to_path4gmns(gmns_db, out_dir=None, mode="driving")` | writes `node.csv` and `link.csv` of the GMNS database (DuckDB writes them) and loads them in path4gmns (`read_network`, metres and km/h): the path4gmns network. The node ids are the OSM node ids; the link ids are the `edge_id` | yes |
| `to_aequilibrae(net, project_dir, mode=None)` | a new AequilibraE project: one link for each edge (`direction` 1), `speed_ab` (km/h), `capacity_ab` (lanes × the capacity of a lane of the class, as in GMNS), `travel_time_ab` (minutes), the mode (`c`, `w`, `b`), a `link_type` for each OSM `highway`, the field `edge_id`. AequilibraE makes the nodes. Returns `{path, links, nodes}` | yes |

```python
net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")   # all modes; net.driving.to_gmns(...) for one
pnet = duckosm.planning.to_path4gmns("out/gmns.duckdb")
duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")
```

| Extra | Installs |
|---|---|
| `duckosm[planning]` | path4gmns (and requests, which path4gmns needs) |
| none | AequilibraE: `pip install aequilibrae` (Python 3.11 or later) and SpatiaLite (`apt install libsqlite3-mod-spatialite`). AequilibraE 1.7 needs pandas 3, so it is not in an extra |

Notebook: [8. Transport planning](../../notebooks/08_planning.md). Guide:
[duckOSM for transport planning users](../../compare/for-planning-users.md).

Pages: [GMNS](../../exports/gmns.md), [GMNS meso and micro](../../exports/gmns-meso-micro.md).
