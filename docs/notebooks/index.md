# Notebooks

Six Jupyter notebooks show duckOSM from Python. Each notebook uses Monaco, installs duckOSM when it is not installed,
and runs in less than 2 minutes. The pages show the outputs of a run on Monaco. A map on a page is a picture; when you
run the notebook, you get the live map.

| Notebook | What you learn | Run |
|---|---|---|
| [1. From an area to a network](01_area_to_network.md) | Choose an area on a map, get its OSM file, build the network, query the edges with SQL | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/01_area_to_network.ipynb) |
| [2. Fix the OSM data once, for every output](02_osm_fixes.md) | Change one OSM tag in the fixes file, build again, and see the fix in the route, the map and six outputs | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/02_osm_fixes.ipynb) |
| [3. Routing](03_routing.md) | Routes between roads, between points and across modes; directions; networkx and rustworkx graphs | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/03_routing.ipynb) |
| [4. Exports](04_exports.md) | SUMO, GMNS, MATSim, OpenDRIVE, GeoPackage, the QuackOSM table and PMTiles, each with a check | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/04_exports.ipynb) |
| [5. Join your data by edge_id](05_join_by_edge_id.md) | Join traffic counts to the edges by `edge_id`, color the map by them, and keep the join after a rebuild | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/05_join_by_edge_id.ipynb) |
| [6. Street network analysis](06_analysis.md) | The numbers of OSMnx in two views, junctions, street directions, connected parts, nearest roads, clip, random points, OSMnx on the network | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/06_analysis.ipynb) |

## Run a notebook on your computer

```bash
pip install "duckosm[all]" jupyterlab
jupyter lab
```

The notebooks write their files (`data/`, `out/`) in the folder of the notebook. Each one needs a connection to the
internet: it downloads the Monaco extract from Geofabrik.

The CI workflow runs the notebooks on each change of the code or of a notebook. A failed cell fails the workflow.
