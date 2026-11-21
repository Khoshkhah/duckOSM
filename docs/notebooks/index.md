# Notebooks

**Read the chapters as a book:** [the duckOSM notebook book](https://khoshkhah.github.io/duckOSM/book/) has the
chapters in the book style, with numbered sections, boxes and a light and a dark theme. Each chapter has a button to
run it in Colab.

The Jupyter notebooks show duckOSM from Python. Each notebook uses Monaco, installs duckOSM when it is not installed,
and runs in less than 2 minutes. The pages show the outputs of a run on Monaco. A map on a page is a picture; when you
run the notebook, you get the live map.

## Start with the core

The core: get a network, query it, draw it, join data by `edge_id` and export it. Start with notebook 1.

| Notebook | Module | What you learn | Run |
|---|---|---|---|
| [1. From an area to a network](01_area_to_network.md) | core | Choose an area on a map, get its OSM file, build the network, query the edges with SQL | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/01_area_to_network.ipynb)<br><a href="01_area_to_network.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [4. Exports](06_exports.md) | core | SUMO, GMNS, MATSim, OpenDRIVE, GeoPackage, the QuackOSM table and PMTiles, each with a check | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/06_exports.ipynb)<br><a href="06_exports.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [5. Join your data by edge_id](04_join_by_edge_id.md) | core | Join traffic counts to the edges by `edge_id`, color the map by them, and keep the join after a rebuild | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/04_join_by_edge_id.ipynb)<br><a href="04_join_by_edge_id.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |

## One notebook for each module

Each module is for one group of users ([the modules](../reference/python-modules.md)). The order is the order of the modules.
`duckosm.routing` has two notebooks: the basics (3) and the `Route` object with its analyses (11).

| Notebook | Module | What you learn | Run |
|---|---|---|---|
| [12. Street network analysis](09_analysis.md) | `duckosm.analysis` | The numbers of OSMnx in two views, junctions, street directions, connected parts, nearest roads, clip, random points, OSMnx on the network | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/09_analysis.ipynb)<br><a href="09_analysis.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [6. OSM data for GIS](10_gis.md) | `duckosm.gis` | OSM features as GeoDataFrames (as OSMnx's `features_from_*`), the network as OSM XML, the administrative areas | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/10_gis.ipynb)<br><a href="10_gis.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [3. Routing](07_routing.md) | `duckosm.routing` | Routes between roads, between points and across modes; directions; networkx and rustworkx graphs | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/07_routing.ipynb)<br><a href="07_routing.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [11. Advanced routing](08_routing_advanced.md) | `duckosm.routing` | The `Route` object, many routes, the k best routes, a travel time matrix, isochrones and accessibility, each with a map or a chart | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/08_routing_advanced.ipynb)<br><a href="08_routing_advanced.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [7. Traffic simulation](12_sim.md) | `duckosm.sim` | The SUMO network read with sumolib, a short SUMO run, and its counts joined back to the edges by `edge_id`; MATSim links | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/12_sim.ipynb)<br><a href="12_sim.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [13. Mobility data](11_mobility.md) | `duckosm.mobility` | Map matching of GPS traces onto `edge_id`, sensors on the edges, counts per edge and hour, the old edge ids after a rebuild | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/11_mobility.ipynb)<br><a href="11_mobility.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [8. Transport planning](13_planning.md) | `duckosm.planning` | The GMNS network and its checks, loaded in path4gmns; the same shortest path in duckOSM and path4gmns | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/13_planning.ipynb)<br><a href="13_planning.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [9. HD maps for automated driving](14_hdmap.md) | `duckosm.hdmap` | The lanes as a CommonRoad map (read back with commonroad-io), a Lanelet2 map and an OpenDRIVE file | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/14_hdmap.ipynb)<br><a href="14_hdmap.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [10. Graph machine learning](15_ml.md) | `duckosm.ml` | The junction graph and the turn graph as PyTorch Geometric graphs; a small graph neural network that learns the speed limits | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/15_ml.ipynb)<br><a href="15_ml.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |
| [2. Fix the OSM data once, for every output](05_osm_fixes.md) | `duckosm.fixes` | Change one OSM tag in the fixes file, build again, and see the fix in the route, the map and six outputs | [![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/05_osm_fixes.ipynb)<br><a href="05_osm_fixes.ipynb" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a> |

## Run a notebook on your computer

```bash
pip install "duckosm[all]" jupyterlab
jupyter lab
```

The notebooks write their files (`data/`, `out/`) in the folder of the notebook. Each one needs a connection to the
internet: it downloads the Monaco extract from Geofabrik.

The CI workflow runs the notebooks on each change of the code or of a notebook. A failed cell fails the workflow.
