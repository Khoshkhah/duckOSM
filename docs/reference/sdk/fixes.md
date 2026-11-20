# duckosm.fixes

Edit the OSM fixes from Python. The module comes in PR 6 of the design; it is empty today. Until then, write the fixes
file by hand or on a map: [Fix the OSM data](../../guides/edit-osm-data.md).

| Call | Does | Today |
|---|---|---|
| `net.fixes` | the fixes file that the network was built with | PR 6 (today: the card of the network names it) |
| `Fixes(path)` | opens or makes a fixes file | PR 6 |
| `fixes.set(way=..., maxspeed=30)`, `.remove(way=...)`, `.exclude(way=..., modes=[...])` | adds a rule | PR 6 |
| `net.rebuild(fixes=fixes)` | builds the area again | PR 6 |
| `net.diff(other)` | the edges that changed between two networks | PR 6 |
| `net.edit()` | opens `duckosm edit` on the file | PR 6 |
