# duckOSM Documentation

Welcome to duckOSM - a high-performance OSM-to-routing-network converter.

## Documentation

- [User Manual](user_manual.md) - Installation, CLI usage, API, routing & SUMO export
- [GIS Export](gis_export.md) - GeoPackage / shapefile network export (`duckosm export-gis`)
- [GMNS Export](gmns_export.md) - standalone GMNS DuckDB with lane detail & movements (`duckosm gmns`)
- [GMNS Meso](gmns_meso.md) - mesoscopic lane-level network (section + turn-connector links) (`duckosm gmns --meso`)
- [GMNS Viewer](gmns_viewer.md) - interactive HTML viewer for a GMNS DuckDB (lanes / meso / micro, hover tooltips) (`duckosm gmns-viz`)
- [GMNS Maps](gmns_map.md) - pretty road-by-direction / lane-width HTML maps of a GMNS DuckDB (`duckosm gmns-map`)
- [MATSim Export](matsim_export.md) - MATSim network.xml (nodes + directed links) for MATSim / BEAM / eqasim (`duckosm matsim`)
- [MATSim Lanes & Signals](matsim_lanes_signals.md) - turn lanes.xml + signalSystems/Groups/Control from a GMNS db (`duckosm matsim-lanes`)
- [OpenDRIVE Export](opendrive_export.md) - ASAM OpenDRIVE .xodr (roads + lane-level geometry) for AV sims / commercial micro (`duckosm opendrive`)
- [railML Export](railml_export.md) - railML 2.4 rail infrastructure (tracks, switches, signals, OCPs) for OpenTrack / RailSys (`duckosm railml`)
- [Multi-Mode Support](multi_mode.md) - Driving, Walking, and Cycling modes
- [Multimodal Routing](multimodal.md) - Design spec: intermodal trips (walk→drive→walk) across modes
- [Feature Schema](features_schema.md) - Base-map `features.*` (Shortbread layers) for rendering; the duckmap migration
- [Data Dictionary](data_dictionary.md) - Table and schema descriptions
- [Query Cookbook](query_cookbook.md) - SQL snippets for analysis
- [Architecture](architecture.md) - Pipeline design and processor details
- [Walkthrough](walkthrough.md) - Development notes and technical details

## Quick Start

```bash
# Install
pip install -e .

# Run
duckosm build --pbf input.osm.pbf --output network.duckdb
```

## Performance

| Dataset | Nodes | Edges | Time |
|---------|-------|-------|------|
| Somerset | 13K | 3.4K | 0.22s |
| GTA | 2.4M | 1M | 5.68s |
