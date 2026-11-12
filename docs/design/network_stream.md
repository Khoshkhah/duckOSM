# The network step in limited memory

Status: proposal, 2026-11-11. Part A is the first work. The load of `raw.*` (`read_osm`) and the global junctions are
done before it: see [streaming_load.md](streaming_load.md).

## The problem

The network step (`duckosm_core::network::build_network_junctions`) keeps all the data of a mode in RAM until the
end: the ways, their coordinates, the filtered ways, the edges and the edge graph. A France build (5.1 GB file,
driving) went above 26 GB and did not finish on a computer with 23 GB of RAM (2026-11-11).

## The measurements

`rust/duckosm-core/examples/mem_probe.rs` on a Mac mini (Apple Silicon, 12 cores, 16 GB), 2026-11-11. The live heap is
counted by a counting allocator. The time is the time of each part (one thread).

Kentucky, driving (2,114,699 edges, 7.5 vertices for each edge; peak RSS 2.49 GB, 7.2 s):

| Sub-step | Time | Memory added (live heap) |
|---|---|---|
| 0a Read the highway ways from the PBF (`osmpbf`) | 1.55 s | +179 MB |
| 0b Read the coordinates of their nodes (`osmpbf`) | 2.03 s | +192 MB (and +181 MB for a moment) |
| Prep: global and local junctions, the mode filter | 0.40 s | +379 MB |
| 1 Cut the ways into forward edges | 0.56 s | +509 MB |
| 2b, 3, 4 Merge the chains, split parallel and antiparallel arcs | 0.28 s | 0 (+190 MB for a moment) |
| 5, 6, 7 Reverse edges, `edge_id` and `edge_ref`, length | 0.49 s | +362 MB |
| 8, 8b, 8c, 9 Nodes, path connectors, speeds, private roads | 0.30 s | +23 MB |
| 9b, 9b2 Turn restrictions, edge graph | 1.03 s | +241 MB |
| 10 Keep the largest connected part | 0.37 s | -40 MB (+170 MB for a moment) |

Estonia, walking (1,394,642 edges; peak RSS 1.38 GB, 4.4 s) has the same shape: reading 2.5 s, prep +190 MB, cut
+226 MB, edge graph 0.5 s, largest part 0.2 s.

At the end, the network holds: the edges 801 MB (397 bytes for each edge: a 160-byte `Edge`, the node ids and the
coordinates), the edge graph 241 MB, the tables `ways` and `way_nodes` 178 MB, the coordinates 192 MB and the ways as
read 179 MB.

Three facts from the measurements:

1. Reading the PBF with `osmpbf` is half of the time. `pbfraw` reads it 5 times faster, and `raw.*` is already in
   DuckDB after `read_osm`.
2. The sub-steps use one thread. With 12 threads, only the reading is faster (Kentucky: 7.2 s to 5.3 s).
3. The memory grows in four places, and each one keeps all ways or all edges of the mode: the prep, the cut, the
   reverse edges and the edge graph.

## The data that each sub-step reads

| Data | Read by |
|---|---|
| All ways as read (`all_ways`) | the global junctions, the mode filter, step 6 (`edge_ref` needs the node list of the raw way) |
| The filtered ways (`mw`) | the cut, step 5 (the lanes and the type of the reverse edge, the bus lane), step 9b (turn restrictions), the output `ways` and `way_nodes` |
| The coordinates (`coords`) | the cut, step 2b (the merge of chains), step 8 (the position of each node) |
| The forward edges, then the edges | steps 2b to 10 |
| Outside the network step | the area tables, the levels and the lane profile read the ways and the coordinates again |

So a stream of sub-steps 0 and 1 alone frees nothing: the later steps keep the ways, the filtered ways and the
coordinates in RAM. Part A must also give the edges the values that the later steps read through these data.

## The target design

All data is on disk (DuckDB tables or Parquet files), and the network step is a series of passes. Each pass works on
one part of the data at a time. Most sub-steps need only one way, one edge, one node or one key:

| Kind of pass | Sub-steps | Part of the data |
|---|---|---|
| By way | 0, the mode filter, 1, 5, 6, 7, 8c, 9 | a batch of ways, with the junction flags of their nodes |
| By key | 3, 4 | the edges with the same end nodes |
| By node | 8, 9b2 | the edges at a node |
| Global, small | 2b, 10 | only the ids and the end nodes of the edges (a union-find), not the geometry or the tags |
| By tile, with a margin | 8b | the edges near a point (walking and cycling only) |

The `edge_id` is a hash of the way and its two end nodes, so it does not depend on how the work is divided.

## Part A: the ways in a stream, the edges on disk

The input is `raw.ways`, `raw.nodes` and `main.global_junctions` in DuckDB. For a whole-file build of `duckosm-rust`,
they are there before the network step (`read_osm` and the global junctions SQL). This needs one change of the order
of the build: today, the Rust build makes the network first and the `.duckdb` file after it.

1. **Pass 1, the mode filter (Rust, by way).** DuckDB gives the highway ways with their tags. Each way goes through
   `filter::mode_way` and the per-mode OSM fixes. The pass writes the ids of the kept ways, their road flag, and the
   tables `ways` and `way_nodes` of the mode (they no longer stay in RAM).
2. **The local junctions (SQL).** The same rule as `local_junctions`, on the kept ways, as the global junctions do.
3. **Pass 2, the cut (Rust, by way, on all threads).** DuckDB gives the kept ways with the coordinates of their nodes
   and the junction flags (a join, which DuckDB can do with limited memory). Each way is cut into forward edges. The
   edge gets the values that steps 5 and 6 read today through `mw` and `all_ways`: the lanes and the type of the
   reverse direction, the bus-lane flag and the position of the edge in the raw way. The forward edges go to a file.
4. **The bridge.** Steps 2b to 10 read the forward edges back into RAM and run as today. Step 9b reads only the ways
   that a turn restriction names. Step 8 takes the node positions from the edges.

After part A, the RAM of the network step holds only the edges, not the ways, the filtered ways and the coordinates.
My estimate for Kentucky: the peak goes down by about 0.5 to 0.75 GB of 1.9 GB (live heap). This is an estimate; part A
measures it.

### Step A1 (done): `raw.*` first, the input from DuckDB

A whole-file build of `duckosm-rust` with the `.duckdb` output now writes `raw.*` with `read_osm` first. The network
reads the highway ways, the coordinates of their nodes and the turn restrictions from `raw.*`
(`duckosm_duckdb::RawInput`, through the trait `NetworkSource` and `Input::Source`), not from the file a second time.
`write_network_tables` adds the network tables to the file. A build of an area is the same as before.

Measured on the Mac mini, against `main`:

| Area | main | A1 | Tables |
|---|---|---|---|
| Monaco, 3 modes | 0.88 s, 0.15 GB | 0.28 s, 0.15 GB | 38 equal |
| Estonia, 3 modes | 23.5 s, 3.45 GB | 23.3 s, 3.98 GB | 38 equal |
| Kentucky, driving | 17.3 s, 4.38 GB | 18.2 s, 4.49 GB | 20 equal |

"Equal" means every row of every table (`EXCEPT ALL` in both directions), except the build time in
`main.visualization_metadata` and `visualization.edge_levels_meta`. The memory is the same within the run-to-run range of
macOS (about 0.4 GB), as expected: A1 changes the order and the source, not what stays in RAM. On 12 threads the reader of
the file is already fast (Kentucky: 0.91 s; from `raw.*`: 0.97 s).

The step times of Kentucky (`--timing`) show one more cost: the network tables go to DuckDB through GeoParquet files.
Writing the files takes 3.2 s and DuckDB reads them again in 3.1–3.4 s, together 37% of the build (17.6 s). Part H below.

### The checks of part A

- Every table of the network step is equal to the output before part A: the golden tests on Monaco and Tartu, and a
  comparison of all rows (`EXCEPT ALL` in both directions) of the edges, the nodes and the edge graph on Estonia and
  Kentucky.
- `mem_probe` and the peak RSS on Kentucky and Estonia, before and after, on the Mac mini, several runs (the peak RSS
  of macOS changes by about 0.4 GB from one run to the next).

## Part C (done): the edge graph written while it is made

The rows of `edge_graph` go to the file writer while the network step makes them (`graph::edge_graph_each` and
`duckosm_parquet::EdgeGraphWriter`). The network does not keep them. To make this possible:

- The largest part (step 10) runs before the edge graph (step 9b). The graph is then made only for the kept edges, and
  `Graph::prune` is not necessary.
- The edges are sorted by `edge_id` before the edge graph. `edge_graph_each` gives the rows of each edge in the order of
  `to_edge`, so the file has the rows sorted by `(from_edge, to_edge)`, as before, with no sort of all rows.
- The edges that start at a node are one slice of a compact list (source, `edge_id`, label, bridge, heading), sorted by
  `(source, edge_id)`, with a hash map from the node to its slice. A binary search for each edge in the order of
  `edge_id` reads about 20 random places, and made the step 0.9 s slower on Kentucky. With the hash map it is faster
  than before.
- `edge_graph` (the function that keeps the rows, for the browser and the tests) collects the rows of `edge_graph_each`.

The feature `heap-probe` of `duckosm-cli` counts the bytes that the program has from the allocator, and prints the live
and the peak bytes and the time at each step. The RSS of macOS is not a good measure, because the allocator keeps free memory.

Measured on the Mac mini, against `main`:

| Kentucky, driving | main | part C |
|---|---|---|
| Network step (`build driving`) | 3.20 s | 3.07 s |
| Write the network files | 3.26 s | 2.98 s |
| Live heap after the network step | 1741 MiB | 1497 MiB |
| Peak heap in the network step | 1918 MiB | 1803 MiB |
| Peak heap of the build (`write the area tables`) | 2970 MiB | 2726 MiB |

All tables are equal on Monaco (38), Estonia (38) and Kentucky (20), with the same comparison as in part A. The build
peak is now in `write the area tables`, which part F moves to DuckDB.

## The levels take the edges (done)

After the network files of a mode are written, the levels made a copy of the geometry of all edges, while the network
was still in memory: this was the peak of the build. Now `LevelEdges::add` takes the edges, and their geometry moves into
the level edges. The tags (`Arc<Attrs>`) are shared and only the five tags that the levels read are copied. The bus
routes, which read the edges, run before.

| Kentucky, driving | before | after |
|---|---|---|
| Peak heap of the build | 2726 MiB | 2347 MiB |
| Live heap after the network | 1767 MiB | 1769 MiB |

The live heap after the network (1.77 GB: the ways, the coordinates, the level edges, the lane edges and the nodes) stays
until the levels and the lane profile are written. Part F reads them from DuckDB.

## Part B: the edges in temporary files

The measurements show that the edges of a mode are the largest part of the memory (Kentucky: 1.1 GB of 1.5 GB after the
network step). A gain comes only when no step holds all edges, so part B changes the steps one after the other. Until
the last stage, the peak does not go down: the steps that are not changed still hold all edges.

**Where the edges go.** The temporary files are Rust files of rows (`duckosm_core::spill`), not DuckDB tables. The
reason: the same code must run in the browser, where Rust cannot call DuckDB-Wasm, and a file of rows needs no
conversion of each row. The browser gives a store in OPFS; until then its files are in memory. Only the result tables
are Parquet.

| Stage | Work |
|---|---|
| B1 (done) | `spill`: files of rows, `EdgeCodec` (the shared tags are stored one time), and a stable sort with a memory budget (sorted blocks, then a merge) |
| B2 (done) | Steps 1 to 7 as passes: the cut writes the forward edges to a file; 2b finds the chains with a sort of the edge ends and a union-find on edge numbers, then sorts the edges by chain; 3 to 7 read the edges sorted by `osm_id`, one way at a time |
| B3 | Steps 8 to 10 as passes: the nodes from a sort of the edge ends; the speeds and the private roads edge by edge; the largest part with a union-find on node ids |
| B4 | The edge graph and the writers read the file sorted by `edge_id`; the edge graph keeps only its compact list of the next edges |
| B5 | The steps after the network (levels, lane profile, bus routes, street segments) read the file. The levels of roadstyle (`roadstyle-levels`) hold the geometry of all edges of all modes: this needs a change in roadstyle |
| B6 | The path connectors (8b, walking and cycling) by tile with a margin (part E) |

Each stage keeps the output equal (all tables, `EXCEPT ALL`) and is measured with the heap probe on Kentucky.

### Stage B2 (done): steps 1 to 7 as passes

The steps 1 to 7 of `build_network_junctions` keep no edges in memory. The store is a temporary folder natively
(`spill::temp_store`) and memory in WebAssembly. Each sort has a budget of 64 MiB.

1. **The cut (steps 1 and 2)** writes each forward edge to a file (`Fwd`: the edge, the index of its way and its `Pos`,
   below). For the merge of chains, the two ends of each edge go to a second file.
2. **The merge of chains (2b)** sorts the ends by node, and finds the contraction nodes with the same rule as before. All
   edges at a contraction node join one set (a union-find on the edge numbers: 4 bytes and 1 byte for each edge). The
   edges at no contraction node go to the result as they are. The other edges go to a file sorted by their set, and
   each set is merged in memory with the walk of before. A set holds all edges that one walk can visit, so the chains
   are the same.
3. **Steps 3 to 7** read the edges sorted by `(osm_id, Pos)`, one way at a time. The keys of steps 3, 4 and 6 contain
   `osm_id`, so each way is independent.
4. **The bridge to step 8.** Step 8 and later still take a `Vec<Edge>` in the old order. The bridge reads the file into
   memory and sorts it there. Part B3 removes the bridge.

**The order of before.** These results depend on the order of the old list of all edges: the tie-break of step 3, the
order of the chains, of the virtual nodes and of `edge_id_map`, and the order of the edges at step 8 (the tie rules of
step 8b and of the largest part in step 10). Each edge carries `Pos`, its place in the old list: after 2b the merged
chains in the order (first edge, start, end, order of the walk), then the other edges in the order of the cut, and
`half` for the halves of a split edge. A virtual node gets the step that made it and the `Pos` of its edge.

**A change in `spill`.** `Codec::put` takes `&self` (the codec has its tables in a `RefCell`), so that a pass reads one
file and writes the next with the same codec. The budget of `sort` now counts the memory of each row in a block (its
bytes, its key and its place), not only its bytes in the file. Before, the sort of the 2.4 million edge ends of
Kentucky (21 bytes each) used about 210 MiB with a budget of 64 MiB.

Measured on the Mac mini, Kentucky, driving (`--h3-resolution 8 --timing`, heap probe, three runs; the times are the
middle values):

| Kentucky, driving | main | B2 |
|---|---|---|
| Step 1, the cut | 0.57 s | 0.72 s |
| Step 2b, the merge of chains | 0.09 s | 0.21 s |
| Steps 3 to 7 (B2: with the sort by `osm_id`) | 0.69 s | 0.91 s |
| The bridge to step 8 | | 0.32 s |
| Network step (`build driving`) | 3.07 s | 3.88 s |
| Live heap during steps 1 to 7 | up to 1484 MiB | 748 MiB |
| Peak heap during steps 1 to 7 | 1680 MiB | 860 MiB |
| Live heap after step 7 | 1484 MiB | LIVE7 MiB |
| Peak heap of the network step | 1803 MiB | NETPEAK MiB |
| Peak heap of the build | 2348 MiB | BUILDPEAK MiB |
| Build | 17.6 s | 18.0 s |

B2 alone gives no memory gain, as expected: the bridge makes the list of all edges again, and steps 8 to 10 and the edge
graph hold it. But during steps 1 to 7 the live heap holds only the ways, the coordinates and the junctions (748 MiB):
when the later steps are passes too, the peak goes down. The network step is 0.8 s slower: each pass writes and reads
the edges, and a sort decodes each row to get its key (one time in a block, one time more in the merge of the blocks;
the merges cost about 0.25 s).

All tables are equal on Monaco (38), Estonia (38) and Kentucky (20), with the comparison of part A. With
`--no-global-junctions` too (37, 37 and 19 tables): with the global junctions there is almost no contraction node, so
only this build tests the merge of chains (Kentucky: 51,450 rows of `edge_id_map`).

**What B3 needs.**

- Steps 8 to 10 read the file of edges, not a `Vec`. The tie rules that use the order of the list must use `Pos`
  directly: the highway of a segment in step 8b (the first edge of the segment), the order of the nodes (the tie of the
  nearest node in step 8b) and the root of the largest part in step 10. Then the bridge goes away.
- The nodes come from a sort of the edge ends. The virtual nodes already carry their order.
- A sort decodes the whole row to get its key. With more passes, a key that a codec reads from the start of the row,
  with no decode of the geometry, can save time. Measure it first.
- The merge of chains reads the tags of the ways (`mw`) by the way index, to compare two edges. Part A removes `mw`
  from memory; then the end rows must carry a number for the tags of the road.

## The later parts

| Part | Work | Gain (to measure) |
|---|---|---|
| B | The edges in temporary files (above) | the edges of a mode (Kentucky: 1.1 GB) |
| C | Done (above) | |
| D | The merge of chains and the largest part as union-finds on ids only | the global steps without the edges in RAM |
| E | The path connectors (8b) by tile with a margin | the last step that reads all edges |
| F | The steps after the network (area tables, levels, lane profile) read DuckDB, not the ways in RAM | |
| G | The browser (way C): the same passes, with DuckDB-Wasm and the Parquet files in OPFS | |
| H | The network tables go into DuckDB directly, not through GeoParquet files (as `read_osm` does for `raw.*`) | Kentucky: up to 6.4 s of 17.6 s |

Each part is one pull request with the same checks as part A.
