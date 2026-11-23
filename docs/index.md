# duckOSM

**Turn OpenStreetMap into routable networks, GMNS models, and GIS layers. One Rust engine powered by DuckDB, for your computer and your browser.**

duckOSM reads an `.osm.pbf` file and builds the complete transport network of an area—from GIS layers and turn-by-turn routing to multi-scale GMNS models, traffic simulations, and Graph Neural Networks. It processes areas tile by tile, keeping memory low and speeds fast even for an entire country. Fix the OSM data once on a map: your fixes go into one YAML file and apply at build time, so every downstream table, network layer, and export inherits the same corrected data.

<div class="hero-actions" markdown>

[Try it in your browser](https://khoshkhah.github.io/duckosm-studio/){ .md-button .md-button--primary }
[Install](install.md){ .md-button }
[Get started](first_network.md){ .md-button }
[Notebook manual](https://khoshkhah.github.io/duckOSM/book/){ .md-button }
[Compare](compare/index.md){ .md-button }

</div>

<figure markdown="span">
  ![duckOSM Studio: the Build card in the panel, and the road network of Monaco on the map](images/tour-studio.png)
  <figcaption>duckOSM Studio in the browser: choose an area, build its network, edit the OSM data and export.</figcaption>
</figure>

## What duckOSM makes

All outputs come from one `.osm.pbf` file, in five levels, and all take your OSM fixes. Each output is checked against a reference tool ([how](concepts/checks.md)).

<div class="dk-levels">
<a class="lv" href="exports/osm-file/"><span class="n">0</span><b>OSM file</b><span class="db-tag">.osm.pbf</span><span class="desc">Exact boundary cut with your fixes</span><span class="tools">pbfcut</span></a>
<a class="lv" href="exports/osm-data/"><span class="n">1</span><b>OSM data</b><span class="db-tag">from .osm.pbf</span><span class="desc">Flat tables of all OSM objects</span><span class="tools">QuackOSM · GDAL · Geofabrik · PMTiles</span></a>
<a class="lv" href="exports/routable-network/"><span class="n">2</span><b>Routable network</b><span class="db-tag">DuckDB</span><span class="desc">Drive, walk, bike<br>Edges, edge_id, turns</span><span class="tools">GeoPackage · networkx</span></a>
<a class="lv" href="exports/gmns-network/"><span class="n">3</span><b>GMNS network</b><span class="db-tag">GMNS DuckDB</span><span class="desc">Macro, meso, micro<br>links, lanes, turns</span><span class="tools">GMNS · MATSim · SUMO</span></a>
<a class="lv" href="exports/hd-map/"><span class="n">4</span><b>HD map</b><span class="db-tag">3D geometry</span><span class="desc">Lane shape in 3D (experimental)</span><span class="tools">Lanelet2 · OpenDRIVE</span></a>
</div>

All outputs, by level: [Export](exports/index.md). Speed against other tools: [Compare](compare/index.md).

## Highlights

<div class="grid cards" markdown>

-   :material-road-variant:{ .lg .middle } **Routable networks**

    ---

    Driving, walking and cycling in one DuckDB file, with the graph of legal turns.
    [Build a network](guides/build.md)

-   :material-pencil-ruler:{ .lg .middle } **Fix the OSM data**

    ---

    Edit roads, buildings, nodes and turns on a map. Every output uses your fixes.
    [Edit OSM data](guides/edit-osm-data.md)

-   :material-web:{ .lg .middle } **In the browser too**

    ---

    The same Rust engine runs in a web page, with no install, and gives the same rows.
    [Browser and native](concepts/browser-native.md)

-   :material-identifier:{ .lg .middle } **Edge ids that stay**

    ---

    One id for each road **and each direction**, the same after a rebuild, a cut and an export.
    [Stable edge ids](concepts/edge-ids.md)

-   :material-hub:{ .lg .middle } **GMNS in DuckDB**

    ---

    The open standard (General Modeling Network Specification) for links, lanes, and turning movements.
    The universal bridge from OSM to simulation models and Graph Neural Networks.
    [GMNS network](exports/gmns.md)

-   :material-car-multiple:{ .lg .middle } **Traffic simulation**

    ---

    SUMO, MATSim, and DTALite exports with automated validation. Simplifies continuous roads for
    faster simulation while preserving stable `edge_id`s.
    [Exports](exports/index.md)

-   :material-layers-triple:{ .lg .middle } **GIS layers**

    ---

    GDAL's OSM layers and QuackOSM's table, the same rows, faster, also in the browser.
    [duckOSM and GDAL](concepts/gdal.md)

-   :material-map-search:{ .lg .middle } **Any area, any size**

    ---

    A boundary, a place name, a box or an H3 cell. Countries build by map tiles, in little memory.
    [Prepare an area](guides/prepare-area.md)

-   :material-map-marker-path:{ .lg .middle } **GPS traces on the roads**

    ---

    Map matching with an HMM or with GPS-DTW, then counts and speeds on each edge.
    [duckosm.mobility](reference/sdk/mobility.md)

-   :material-language-rust:{ .lg .middle } **One Rust program**

    ---

    The command line `duckosm`, a Python SDK and a Rust SDK, all from the same code.
    [Python SDK](reference/python.md)

</div>

## A tour

<div class="grid tour" markdown>

<figure markdown="span">
  [![The Turn tab of Edit OSM data: a picked road and the roads that leave its junction](images/edit-turn.png)](guides/edit-osm-data.md)
  <figcaption>Edit OSM data: the turns at a junction, in the browser or with <code>duckosm edit</code>. <a href="guides/edit-osm-data/">Edit OSM data →</a></figcaption>
</figure>

<figure markdown="span">
  [![The quack page: the counts of the five GDAL layers of Monaco and a download button for each layer](images/tour-quack.png)](https://khoshkhah.github.io/duckosm-studio/quack.html)
  <figcaption>The quack page: GDAL's five OSM layers or the QuackOSM table, made in the browser. <a href="https://khoshkhah.github.io/duckosm-studio/quack.html">Open the quack page →</a></figcaption>
</figure>

<figure markdown="span">
  [![The route planner: a walking route from Fontvieille to Larvotto on the map, with the directions](images/tour-route.png)](guides/route.md)
  <figcaption>The route planner of <code>duckosm route-map</code>: a route on the turn graph, with directions. <a href="guides/route/">Routing →</a></figcaption>
</figure>

<figure markdown="span">
  [![The map of duckosm viz in 3D: the buildings of Monaco stand up around the port, with the roads between them](images/tour-map.png)](guides/draw-map.md)
  <figcaption>The map of <code>duckosm viz</code> in 3D: the roads of all modes between the buildings, on the base map of the database. <a href="guides/draw-map/">Draw a map →</a></figcaption>
</figure>

<figure markdown="span">
  [![The lanes of one Monaco junction from the GMNS export: each lane at its width, with its turn arrows](images/tour-lanes-gmns.png)](exports/gmns.md)
  <figcaption>GMNS lane-level geometry: each lane at its physical width with its turning movements. <a href="exports/gmns/">GMNS export →</a></figcaption>
</figure>

<figure markdown="span">
  [![Monaco in 3D on the ground of the area, with hillshade: the hill rises behind the port, and the roads climb it](images/tour-elevation.png)](guides/elevation.md)
  <figcaption>Monaco in 3D on the ground of the area: the hill behind the port and the roads that climb it (<code>duckosm elevation</code>, or the step Ground height in Studio). <a href="guides/elevation/">Elevation →</a></figcaption>
</figure>

</div>

## Use it

<div class="grid cards" markdown>

-   :material-console:{ .lg .middle } **Command line**

    ---

    One program, `duckosm`: `build`, `quack`, `sumo`, `gmns`, `edit` and more, each with `--help`.

    [Command line](reference/cli.md) · [Quick start](install.md#quick-start)

-   :material-language-python:{ .lg .middle } **Python SDK**

    ---

    `pip install duckosm`, then `import duckosm`: build, quack, route and export from Python, all in the Rust engine.

    [Python SDK](reference/python.md) · [Notebook manual](https://khoshkhah.github.io/duckOSM/book/) · [Quick start](install.md#quick-start)

-   :material-language-rust:{ .lg .middle } **Rust SDK**

    ---

    The crates `duckosm-core`, `duckosm-build` and others: the same engine in your own Rust program.

    [Rust SDK](reference/rust.md)

-   :material-view-dashboard:{ .lg .middle } **Dashboards & Studio**

    ---

    duckOSM Studio in the browser, or `net.dashboard()` in Python: live maps, road cards, and 3D layers. Save as offline HTML files that open anywhere without a server.

    [duckOSM Studio](guides/studio.md) · [Draw a map](guides/draw-map.md)

</div>

---

Map data © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright) ([ODbL](https://opendatacommons.org/licenses/odbl/)). Files from [Geofabrik](https://www.geofabrik.de/). Built on [DuckDB](https://duckdb.org/). duckOSM is MIT-licensed.
