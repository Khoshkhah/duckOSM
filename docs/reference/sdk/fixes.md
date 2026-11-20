# duckosm.fixes

Edit the OSM fixes from Python. The file is the fixes file of [Fix the OSM data](../../guides/edit-osm-data.md)
(`osm_fixes.yaml`). Each change is written to the file at once, as `duckosm edit` does.

| Call | Returns |
|---|---|
| `Fixes(path="osm_fixes.yaml")` | the fixes file; made at the first rule. `os.fspath(fixes)` is its path, so each `fixes=` takes it |
| `fixes.set(way, **fields)` | adds or changes the rule of the OSM way `way`. The fields of a rule (`oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer`, `turn_lanes_forward`, `turn_lanes_backward`, `keep_nodes`, `note`) go in the rule; each other keyword is an OSM tag (`maxspeed=30`) |
| `fixes.remove(way)` | removes the way from the build |
| `fixes.exclude(way, modes)` | removes the way from the networks of `modes` |
| `fixes.drop(way)` | deletes the rule of the way |
| `fixes.rules()` | the rules as a DataFrame |
| `fixes` (last line of a cell) | a card with a table of the rules |
| `net.fixes` | the `Fixes` of the file that the network was built with, or `None` |
| `net.rebuild(fixes=None, out=None)` | builds the area again with the same modes and `fixes`, from the OSM file and the boundary beside the database; `out` default `<name>_fixed.duckdb`. Returns the new `Network` |
| `net.diff(other, mode=None)` = `diff(net, other)` | a DataFrame of the edges that are different in `other`, by `edge_id`: `mode`, `edge_id`, `change` (`added`, `removed`, `changed`), `changed` (the columns with a new value) |
| `net.edit(fixes=None, port=8790)` | starts `duckosm edit` on the database in the background and returns the URL of the page |

```python
fixes = duckosm.fixes.Fixes("osm_fixes.yaml")
fixes.set(way=4392632, oneway=True, maxspeed=30)
fixed = net.rebuild(fixes=fixes)
net.diff(fixed)
```

The file is written again at each change: its comments are not kept. The other keys of the file (`new_ways`,
`turn_restrictions`, `extra`) stay. Notebook: [2. Fix the OSM data once](../../notebooks/02_osm_fixes.md).
