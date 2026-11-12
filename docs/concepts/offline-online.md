# Offline and online builds

duckOSM builds an area in two versions. They run the same Rust engine for the network, but they keep their data in
different places:

| | Offline | Online |
|---|---|---|
| Program | `duckosm-rust studio` (Rust only), or `duckosm studio` (the Python package) | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/), a page in the browser |
| Where the build runs | on your computer, as a native program | in the browser, as WebAssembly |
| Storage | the disk of your computer | the browser's disk for the site (OPFS), which is small |
| Memory | the RAM of your computer; DuckDB has its own `memory_limit` | the memory of the tab: at most 4 GB for each WebAssembly module |
| SQL | DuckDB, built into the program | DuckDB-Wasm, a separate module in the page |
| Result | a `.duckdb` file on the disk | tables in DuckDB-Wasm; a `.duckdb` file only when you export |

This page shows each step of a build in each version: **what data the step reads, where that data is (in memory or in
storage), and how the step handles it.** It describes the Rust build. The Python `duckosm build` has the same steps,
but it cuts the file with osmium and loads `raw.*` with DuckDB first: see [How a build works](build.md).

## The signs

| Sign | Meaning |
|---|---|
| **Storage** | on a disk: a file of your computer, or a file in the browser's storage (OPFS) |
| **Memory** | in RAM: the program's memory, or the memory of the tab |
| **Kept to the end** | the data stays in memory until the build ends, also after the step that made it |
| **In blocks** | the step reads or writes a part at a time, so its memory does not grow with the area |
| **Spills** | DuckDB keeps the data in memory up to its limit, and writes the rest to a temporary file |

## The offline build

A build of a whole file with `duckosm-rust`. In the diagrams, a blue cylinder is storage, a red rounded box is memory, and a
plain box is a step.

```mermaid
flowchart TB
    PBF[(".osm.pbf file<br/>[storage]")]

    S1["<b>1. Read the file</b><br/>highway ways,<br/>node coordinates,<br/>turn restrictions<br/>(in blocks)"]
    M1("ways, coordinates,<br/>restrictions<br/>[memory, to the end]")
    S2["<b>2. Sidewalk rule,</b><br/><b>global junctions</b>"]
    M2("junction set<br/>[memory]")
    S3["<b>3. Network of each mode</b><br/>cut, merge, split,<br/>reverse edges, edge_id,<br/>nodes, speeds,<br/>edge graph, largest part"]
    M3("edges, nodes, edge graph<br/>of one mode<br/>[memory, until written]")
    S4["<b>4. Write the network files</b><br/>one mode at a time"]
    S5["<b>5. Area tables,</b><br/><b>levels, lane profile</b>"]
    F1[("GeoParquet files<br/>[storage]")]
    S6["<b>6. Write the .duckdb file</b><br/>DuckDB reads the files<br/>(spills to disk)"]
    S7["<b>7. raw.*</b><br/>read_osm: DuckDB reads<br/>the file, one block<br/>for each thread"]
    DB[(".duckdb file<br/>[storage]")]

    PBF --> S1 --> M1
    M1 --> S2 --> M2
    M1 --> S3
    M2 --> S3
    S3 --> M3 --> S4 --> F1
    M1 --> S5 --> F1
    F1 --> S6 --> DB
    PBF --> S7 --> DB

    classDef disk fill:#3b82f633,stroke:#3b82f6,stroke-width:2px
    classDef mem fill:#ef444433,stroke:#ef4444,stroke-width:2px
    class PBF,F1,DB disk
    class M1,M2,M3 mem
```

## The online build

Studio in the browser. The page always builds an area that you choose inside a region file.

```mermaid
flowchart TB
    REG[("region .osm.pbf<br/>[storage: OPFS]")]

    S0["<b>0. Cut to the area</b><br/>reads the region file<br/>in pieces"]
    M0("ways, coordinates,<br/>restrictions, raw.*<br/>of the area<br/>[tab memory, to the end]")
    S2["<b>2. Sidewalk rule,</b><br/><b>global junctions</b>"]
    S3["<b>3. Network of each mode</b><br/>the same Rust engine"]
    M3("edges, nodes, edge graph<br/>of one mode<br/>[tab memory]")
    S4["<b>4–5. Write the files</b><br/>network, area tables,<br/>levels, lane profile, raw.*"]
    M4("Parquet files as bytes<br/>[tab memory]")
    S6["<b>6. Give the files</b><br/><b>to DuckDB-Wasm</b><br/>each file is copied"]
    M6("DuckDB-Wasm:<br/>files and tables<br/>[its memory, at most 4 GB]")
    SAVE["<b>Save the area</b>"]
    O[("Parquet files<br/>[storage: OPFS]")]
    EXP["<b>Export</b><br/>copy the tables<br/>into a .duckdb file"]
    DL[(".duckdb download<br/>[storage]")]

    REG --> S0 --> M0
    M0 --> S2 --> S3 --> M3 --> S4
    M0 --> S4
    S4 --> M4 --> S6 --> M6
    M4 --> SAVE --> O
    M6 --> EXP --> DL

    classDef disk fill:#3b82f633,stroke:#3b82f6,stroke-width:2px
    classDef mem fill:#ef444433,stroke:#ef4444,stroke-width:2px
    class REG,O,DL disk
    class M0,M3,M4,M6 mem
```

## Each step: where the data is and how it is handled

| Step | Offline | Online |
|---|---|---|
| **The input** | **Storage:** the `.osm.pbf` file on the disk. | **Storage:** the region file in OPFS. It comes one time through the relay, and the next area in the same region needs no download. |
| **0. Cut to the area** | A whole-file build has no cut. An area with a boundary: the Rust cut reads the file and keeps the area in **memory**. | Always. The Rust cut reads the region file **in blocks**, from OPFS, and keeps the ways, the coordinates and `raw.*` of the area in **tab memory, to the end**. |
| **1. Read the file** | The highway ways, the coordinates of their nodes and the turn restrictions: read **in blocks**, kept in **memory to the end**, because the network, the area tables, the levels and the lane profile read them again. | Part of the cut (step 0). |
| **2. Sidewalk rule, global junctions** | Work on the ways in **memory**. The junction set is made one time for all modes, in **memory**. | The same, in tab memory. |
| **3. Network of each mode** | The edges and the nodes of the mode are in **memory** until the mode is written. The rows of the edge graph go to the file while they are made. The largest mode sets the peak. See [step 3 in detail](#step-3-in-detail-the-network-of-a-mode). | The same code, in **tab memory**, inside the 4 GB of the module. The rows of the edge graph go to Parquet bytes in tab memory. |
| **4. Write the network files** | GeoParquet files to **storage**, from the network in memory. | Parquet files as bytes in **tab memory**. |
| **5. Area tables, levels, lane profile** | Read the ways and the coordinates in **memory**; write files to **storage**. | The same, files as bytes in **tab memory**. |
| **6. The tables** | DuckDB reads the GeoParquet files into the `.duckdb` file in **storage**. It uses its `memory_limit` and **spills** what is more. | The page gives each file to DuckDB-Wasm, which takes a **copy** into its own memory. So each file is two times in memory: the page keeps its copy for the download. |
| **7. `raw.*`** | A whole-file build: `read_osm` gives the blocks of the file to DuckDB, which writes the tables to **storage**. The memory is DuckDB's limit and **one block for each thread**. An area: from the cut, through Parquet files. | From the cut, as Parquet bytes in tab memory, then a copy in DuckDB-Wasm. |
| **The base-map layers** | An area: `features.sql` runs in DuckDB on `raw.*`; it **spills**. | `features.sql` runs in DuckDB-Wasm on the `raw_*` files in its memory. |
| **The result** | The `.duckdb` file in **storage**. | The tables in DuckDB-Wasm (**memory**). **Save** writes the Parquet files to **OPFS**; **Export** copies the tables into a `.duckdb` file to download. |

## Step 3 in detail: the network of a mode

The network step runs one time for each mode (driving, walking, cycling). The offline and the online build run the same
Rust code. Only the place of the files is different: offline they go to the disk, online they stay as bytes in the
memory of the tab.

```mermaid
flowchart TB
    IN("ways, coordinates,<br/>junction set<br/>[memory, to the end]")
    S0["<b>Mode filter</b><br/>ways of the mode,<br/>OSM fixes,<br/>local junctions"]
    S1["<b>1. Cut</b><br/>each way into<br/>forward edges"]
    M1("forward edges<br/>[memory]")
    S2["<b>2b. Merge chains</b><br/><b>3–4. Split arcs</b><br/>parallel and<br/>antiparallel"]
    S5["<b>5. Reverse edges</b><br/><b>6. edge_id</b><br/><b>7. Lengths</b>"]
    M5("all edges and nodes<br/>of the mode<br/>[memory, until written]")
    S8["<b>8. Nodes</b><br/><b>8b. Path connectors</b><br/><b>8c. Speeds</b>"]
    S9["<b>9. Private roads</b><br/><b>10. Largest part</b><br/><b>9b. Turn restrictions</b>"]
    S9b["<b>9b2. Edge graph</b><br/>the moves of each edge,<br/>in edge_id order"]
    W["<b>Write the</b><br/><b>network files</b>"]
    LV["<b>Levels</b><br/>take the edges<br/>(no copy)"]
    F1[("offline:<br/>GeoParquet files<br/>[storage]")]
    F2("online:<br/>Parquet bytes<br/>[tab memory]")

    IN --> S0 --> S1 --> M1 --> S2 --> S5 --> M5
    M5 --> S8 --> S9 --> S9b
    S9b -- "rows, while made" --> W
    M5 -- "after the mode" --> W
    W --> F1
    W --> F2
    W --> LV

    classDef disk fill:#3b82f633,stroke:#3b82f6,stroke-width:2px
    classDef mem fill:#ef444433,stroke:#ef4444,stroke-width:2px
    class F1 disk
    class IN,M1,M5,F2 mem
```

The memory of each sub-step, measured on Kentucky (driving, 2.1 million edges) on a Mac mini. The values are the live
Rust heap after the sub-step (cargo feature `heap-probe` of `duckosm-cli`):

| Sub-step | What it reads | What it keeps | Kentucky, change of live heap |
|---|---|---|---|
| Mode filter, local junctions | the ways | the ways of the mode, the local junctions | +347 MiB |
| 1. Cut | the ways of the mode, the coordinates, the junctions | the forward edges, with their nodes and geometry; the local junctions are free after it | +509 MiB |
| 2b. Merge chains | the forward edges at each node | longer edges, the map of the old edge ids | 0 (peak +155 MiB) |
| 3–4. Split arcs | the edges with the same way and end nodes | the edges, some cut in two | −142 MiB |
| 5. Reverse edges | each two-way edge | a reversed copy of the edge | +362 MiB |
| 6–8c. edge_id, lengths, nodes, connectors, speeds | each edge; the edges at each node | the nodes; values on the edges | +23 MiB |
| 9, 10, 9b. Private roads, largest part, turn restrictions | the end nodes of all edges | fewer edges | −7 MiB |
| 9b2. Edge graph | the edges that start at each node | nothing: the rows go to the file | 0 (peak +303 MiB) |

After the mode, 1.5 GB is live on Kentucky: the ways and the coordinates (0.4 GB, kept for the next modes and the
later steps) and the edges and the nodes of the mode (1.1 GB). The network files are then written, and the levels take
the edges. The edges are the largest part, so the next work moves them to storage (see
[What changes next](#what-changes-next)).

## The limits today

- **Offline:** the network step. It keeps the ways, the coordinates and all edges of a mode in RAM, so the RAM of the
  computer limits the area. A whole country can be more than the RAM: France (driving) needed more than 26 GB.
- **Online:** the memory of the tab. The cut, the network and DuckDB-Wasm's copy of the files are all in memory, and a
  WebAssembly module has at most 4 GB. The files in OPFS also count against the browser's storage for the site.

## What changes next

The work moves the data from memory to storage, step by step, in the same way in both versions:

1. `raw.*` and the global junctions go into DuckDB **first**, so that the later steps can query them.
2. The network step becomes passes over tables. Each pass reads one part (a batch of ways, the edges at one node, the
   edges with the same end nodes), writes its result as a table, and the next pass queries that table.
3. Online, the Parquet files stay in OPFS, and DuckDB-Wasm reads them from there (it does not copy them into memory).

The design notes are on GitHub: [the load step](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/streaming_load.md)
and [the network step in limited memory](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/network_stream.md).
