# duckOSM Documentation

Welcome to duckOSM - a high-performance OSM-to-routing-network converter.

## Documentation

- [User Manual](user_manual.md) - Installation, CLI usage, and API examples
- [Multi-Mode Support](multi_mode.md) - Driving, Walking, and Cycling modes
- [Data Dictionary](data_dictionary.md) - Table and schema descriptions
- [Query Cookbook](query_cookbook.md) - SQL snippets for analysis
- [Architecture](architecture.md) - Pipeline design and processor details
- [Walkthrough](walkthrough.md) - Development notes and technical details

## Quick Start

```bash
# Install
pip install -e .

# Run
python -m duckosm --pbf input.osm.pbf --output network.duckdb
```

## Performance

| Dataset | Nodes | Edges | Time |
|---------|-------|-------|------|
| Somerset | 13K | 3.4K | 0.22s |
| GTA | 2.4M | 1M | 5.68s |
