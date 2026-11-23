# 2. Routable network

The routable network of each mode: the edges with their `edge_id`, the nodes and the legal turns.

```mermaid
flowchart LR
  db[("<area>.duckdb (DuckDB)")] --> gis["GIS (GeoPackage, shp)"]
  db --> nx["networkx / rustworkx"]
  db --> tr["Turn graph router"]
  db --> rail["railML"]
```

| Read by | Output | Command |
|---|---|---|
| Your own router | [The turn graph](../turn-graph.md): the table `edge_graph` in the database | `route()`, `Router` ([Route](../../guides/route.md)) |
| QGIS, a map style | [Base-map layers](../base-map.md): water, land use, buildings, POIs, in the database | `duckosm build` |
| QGIS, ArcGIS, FME | [GeoPackage or shapefile](../gis.md): the network as GIS layers | `duckosm export-gis` |
| networkx | [networkx](../networkx.md): a networkx graph, GraphML or gpickle | `to_networkx()`, `duckosm export-graph` |
| Rail planning: OpenTrack, RailSys | [railML 2.4](../railml.md) (experimental): the rail tracks, switches and stations, from `raw.*` | `duckosm railml` |
