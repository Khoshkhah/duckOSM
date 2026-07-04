# duckOSM Documentation

Welcome to duckOSM - a high-performance OSM-to-routing-network converter.

## Documentation

- [User Manual](user_manual.md) - Installation, CLI usage, API, routing & SUMO export
- [GIS Export](gis_export.md) - GeoPackage / shapefile network export (`duckosm export-gis`)
- [GMNS Export](gmns_export.md) - standalone GMNS DuckDB with lane detail & movements (`duckosm gmns`)
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
