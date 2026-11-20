# duckosm.planning

Transport planning: GMNS networks with lanes and movements.

| Call | Returns | Today |
|---|---|---|
| `to_gmns(net, out_path, mode=None, modes=None, to_csv=None, lane_geometry=True, combined=False)` | the GMNS tables in a DuckDB file; `to_csv` also writes the CSV files. A `ModeView` or `mode=` gives one mode, else `modes` (default all). | yes |
| `check_gmns(path, modes=None)` | the checks of the GMNS tables: `[(schema, check, bad, total), ...]` | yes |
| `to_meso(gmns_db, ...)`, `to_micro(gmns_db, ...)` | the meso and micro networks | yes |
| `to_aequilibrae(view, project_dir)` | an AequilibraE project | later |
| `to_path4gmns(gmns)` | the input of path4gmns and DTALite | later |

```python
net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")   # all modes; net.driving.to_gmns(...) for one
```

Pages: [GMNS](../../exports/gmns.md), [GMNS meso and micro](../../exports/gmns-meso-micro.md).
