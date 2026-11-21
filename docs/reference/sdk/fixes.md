# duckosm.fixes

Edit the OSM fixes from Python. The file is the fixes file of [Fix the OSM data](../../guides/edit-osm-data.md)
(`osm_fixes.yaml`). The changes stay in memory until `save(path)`. The library chooses no file name, and a load never
writes the loaded file.

| Call | Returns |
|---|---|
| `Fixes(load=None)` | the rules of the file `load` (no file: no rules). `Fixes("f.yaml")` is the same as `Fixes(load="f.yaml")` |
| `fixes.save(path=None)` | writes the rules to `path`. The first save needs a path; a later `save()` writes the same file again. The path can be the loaded file |
| `fixes.set(way, **fields)` | adds or changes the rule of the OSM way `way`. The fields of a rule (`oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer`, `turn_lanes_forward`, `turn_lanes_backward`, `keep_nodes`, `note`) go in the rule; each other keyword is an OSM tag (`maxspeed=30`) |
| `fixes.remove(way)` | removes the way from the build |
| `fixes.exclude(way, modes)` | removes the way from the networks of `modes` |
| `fixes.drop(way)` | deletes the rule of the way |
| `os.fspath(fixes)` | the saved file, or the loaded file when nothing changed, so each `fixes=` takes a `Fixes`. With changes that are not saved, a call stops: "save the fixes first: fixes.save('name.yaml')" |
| `fixes.rules()` | the rules as a DataFrame |
| `fixes` (last line of a cell) | a card with a table of the rules |
| `net.fixes` | the `Fixes` of the file that the network was built with, or `None` |
| `net.rebuild(fixes=None, out=None)` | builds the area again with the same modes and `fixes`, from the OSM file and the boundary beside the database; `out` default `<name>_fixed.duckdb`. Returns the new `Network` |
| `net.diff(other, mode=None)` = `diff(net, other)` | a DataFrame of the edges that are different in `other`, by `edge_id`: `mode`, `edge_id`, `change` (`added`, `removed`, `changed`), `changed` (the columns with a new value) |
| `net.edit(load=None, save=None, port=8790)` | starts `duckosm edit` on the database in the background and returns the URL of the page. The page reads `load` and writes `save`; with no `save`, the first **Save** asks for the file. `fixes=F` is the old name of `load=F, save=F` (a `DeprecationWarning`) |

```python
fixes = duckosm.fixes.Fixes(load="osm_fixes.yaml")
fixes.set(way=4392632, oneway=True, maxspeed=30)       # in memory only
fixes.save("osm_fixes_v2.yaml")                        # osm_fixes.yaml does not change
fixed = net.rebuild(fixes=fixes)
net.diff(fixed)
```

`save` writes the whole file: its comments are not kept. The other keys of the file (`new_ways`,
`turn_restrictions`, `extra`) stay. Notebook: [5. Fix the OSM data once](../../notebooks/05_osm_fixes.md).
