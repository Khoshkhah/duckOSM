"""
Main DuckOSM importer class.
"""

import json
import logging
import time
from pathlib import Path
from typing import Optional

import duckdb

import shutil

from duckosm.config import Config
from duckosm.utils import check_db_name
from duckosm.processors import (
    RoadFilter,
    OsmOverrides,
    GraphBuilder,
    SpeedProcessor,
    CostCalculator,
    RestrictionProcessor,
    H3Indexer,
    EdgeGraphBuilder,
    GraphSimplifier,
    GlobalJunctions,
    FunctionalType,
    PathConnector,
    ComponentFilter,
    DuckdbClipper,
)

logger = logging.getLogger("duckosm")


def pbf_cut_path(cache_dir, name, strategy, pbf, boundary) -> Path:
    """Where the osmium cut of ``pbf`` to ``boundary`` is cached: ``<name>.<strategy>.<key>.osm.pbf``,
    the key a fingerprint of the boundary file's content and the PBF (path, size, modification
    time), so a changed boundary or a newer PBF never reuses an old cut."""
    import hashlib
    st = Path(pbf).stat()
    h = hashlib.sha256(Path(boundary).read_bytes())
    h.update(f"{Path(pbf).resolve()}|{st.st_size}|{st.st_mtime_ns}".encode())
    return Path(cache_dir) / f"{name}.{strategy}.{h.hexdigest()[:10]}.osm.pbf"


class DuckOSM:
    """
    High-performance OSM-to-routing-network converter.
    
    Uses DuckDB's native ST_READOSM for fast PBF parsing and
    SQL-based processing for all transformations.
    """
    
    def __init__(self, config: Config):
        """
        Initialize DuckOSM importer.
        
        Args:
            config: Configuration object
        """
        self.config = config
        self.config.validate()

        if config.source_type == "duckdb":
            self.pbf_path = None
            self.source_db = Path(config.source_db).resolve()
        else:
            self.pbf_path = Path(config.effective_pbf_path).resolve()
            self.source_db = None
        self.output_path = config.get_db_path()
        self.output_path.parent.mkdir(parents=True, exist_ok=True)

        self.con: Optional[duckdb.DuckDBPyConnection] = None
        self.stats = {}
        self.mode_stats = {}
        self.validation_results = {}
    
    def run(self) -> Path:
        """
        Run the full import pipeline.
        
        Returns:
            Path to output DuckDB file
        """
        from rich.console import Console
        from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TimeElapsedColumn
        
        console = Console()
        total_start = time.time()
        
        is_clip = self.config.source_type == "duckdb"
        # Component clean-up only makes sense when clipping to an AREA. A whole-region /
        # whole-country build (no boundary) legitimately has separate components (islands)
        # and 2.35M-edge union-find would be slow — so require a boundary.
        self._do_components = do_components = (self.config.effective_boundary_path is not None) and (
            self.config.clip.keep_largest_component
            or self.config.clip.min_component_edges > 1)
        console.print(f"[bold blue]duckOSM {'Clip' if is_clip else 'Import'}[/bold blue]")
        console.print(f"  Source: {self.source_db if is_clip else self.pbf_path}")
        console.print(f"  Output: {self.output_path}")
        console.print(f"  Modes: {', '.join(self.config.modes)}")
        console.print()

        # 1. Global steps (run once)
        if is_clip:
            global_steps = [
                ("connect", "Connecting to DuckDB", self._connect),
                ("attach_parent", "Attaching parent db", self._attach_parent),
                ("load_boundary", "Loading boundary", self._load_boundary),
            ]
        else:
            global_steps = [("connect", "Connecting to DuckDB", self._connect)]
            # In-pipeline osmium clip: make `boundary` actually clip the graph (A1).
            if self.config.effective_boundary_path:
                global_steps.append(("clip_pbf", "Clipping PBF to boundary", self._clip_pbf))
            global_steps.append(("load_pbf", "Loading PBF file", self._load_pbf))
            if self.config.effective_boundary_path:
                global_steps.append(("load_boundary", "Loading boundary", self._load_boundary))
                if self.config.options.boundary_cells:
                    global_steps.append(("boundary_cells", "Generating boundary cells", self._build_boundary_cells))
            # Mode-agnostic road-junction set, built once from raw.* before the per-mode loop, so every
            # mode segments roads at the same junctions and shares edge_ids. See _build_global_junctions.
            if self.config.options.global_junctions:
                global_steps.append(("global_junctions", "Building global junctions",
                                     self._build_global_junctions))

        # 2. Mode-specific steps (run for each mode)
        def get_mode_steps(mode):
            if is_clip:
                steps = [("clip_mode", f"[{mode}] Clipping from parent", lambda: self._clip_mode(mode))]
                if do_components:
                    steps.append(("component_filter", f"[{mode}] Component clean-up",
                                  lambda: self._component_filter(mode)))
                steps.append(("create_indexes", f"[{mode}] Creating indexes", self._create_indexes))
                if self.config.validation.enabled:
                    steps.append(("validate", f"[{mode}] Validating", lambda: self._validate(mode)))
                return steps

            steps = [
                ("filter_roads", f"[{mode}] Filtering roads", lambda: self._filter_roads(mode)),
            ]
            # Local corrections for known OSM errors — MUST run on `ways` after RoadFilter and
            # before GraphBuilder, because `oneway` is topological (decides the reverse twin).
            if self.config.osm_overrides:
                steps.append(("apply_osm_overrides", f"[{mode}] Applying OSM overrides",
                              lambda m=mode: self._apply_osm_overrides(m)))
            steps.append(("build_edges", f"[{mode}] Building edges", self._build_edges))

            # Cut ways into edges where they meet (the network itself: always runs).
            steps.append(("simplify_graph", f"[{mode}] Simplifying graph", self._simplify_graph))

            # Reconnect dangling cycleway/footway ends to the network BEFORE the component filter, so
            # disconnected paths aren't pruned. Cycling/walking only (driving networks are connected).
            if self.config.clip.connectivity_rescue and mode in ("cycling", "walking"):
                steps.append(("connect_paths", f"[{mode}] Connecting dangling paths",
                              lambda: self._connect_paths(mode)))

            # Flag push-the-bike edges BEFORE speeds — SpeedProcessor keys the walking-speed
            # CASE off the dismount column.
            if mode == "cycling" and self.config.options.cycling_dismount:
                steps.append(("mark_dismount", f"[{mode}] Marking dismount edges",
                              self._mark_dismount))

            if self.config.options.process_speeds:
                steps.append(("process_speeds", f"[{mode}] Processing speeds", lambda: self._process_speeds(mode)))

            if self.config.options.calculate_costs:
                steps.append(("calculate_costs", f"[{mode}] Calculating costs", lambda: self._calculate_costs(mode)))

            # Pedestrian/cyclist functional class (walk_type / cycle_type) from OSM sub-tags.
            if self.config.options.functional_types and mode in ("walking", "cycling"):
                steps.append(("functional_type", f"[{mode}] Deriving functional type",
                              lambda m=mode: self._add_functional_type(m)))

            # Private roads (the mode's effective access is 'private') leave `edges` for
            # `private_edges` before the graph: visible on maps, never routable.
            steps.append(("split_private", f"[{mode}] Moving private roads", self._split_private))

            if self.config.options.extract_restrictions:
                # Turn restrictions only for driving for now
                if mode == "driving":
                    steps.append(("extract_restrictions", f"[{mode}] Extracting turn restrictions", self._extract_restrictions))

            if self.config.options.build_graph:
                steps.append(("build_edge_graph", f"[{mode}] Building edge graph", self._build_edge_graph))
                # Drop boundary-crossing stubs / fragments right after the graph is built (A3).
                if do_components:
                    steps.append(("component_filter", f"[{mode}] Component clean-up",
                                  lambda: self._component_filter(mode)))

            if self.config.options.h3_indexing:
                steps.append(("add_h3_indexing", f"[{mode}] Adding H3 indexing", self._add_h3_indexing))

            steps.append(("create_indexes", f"[{mode}] Creating indexes", self._create_indexes))
            if self.config.validation.enabled:
                steps.append(("validate", f"[{mode}] Validating", lambda: self._validate(mode)))
            return steps

        total_steps = len(global_steps) + sum(len(get_mode_steps(m)) for m in self.config.modes) + 1 # +1 for cleanup
        
        try:
            with Progress(
                SpinnerColumn(),
                TextColumn("[progress.description]{task.description}"),
                BarColumn(),
                TextColumn("[progress.percentage]{task.percentage:>3.0f}%"),
                TimeElapsedColumn(),
                console=console
            ) as progress:
                main_task = progress.add_task("Processing...", total=total_steps)
                
                # Execute global steps
                for name, desc, func in global_steps:
                    progress.update(main_task, description=desc)
                    func()
                    progress.advance(main_task)
                
                for mode in self.config.modes:
                    # Create and switch to schema
                    self.con.execute(f"CREATE SCHEMA IF NOT EXISTS {mode}")
                    self.con.execute(f"USE {mode}")
                    
                    mode_start = time.time()
                    for name, desc, func in get_mode_steps(mode):
                        progress.update(main_task, description=desc)
                        func()
                        progress.advance(main_task)
                    
                    # Capture mode-specific stats
                    self.mode_stats[mode] = {
                        'node_count': self.stats.get('node_count', 0),
                        'way_count': self.stats.get('way_count', 0),
                        'edge_count': self.stats.get('edge_count', 0),
                        'edge_graph_count': self.stats.get('edge_graph_count', 0),
                        'total_time': time.time() - mode_start
                    }

                # Multimodal (intermodal) transfer graph — stitch the per-mode networks into one
                # layered graph so a trip can switch mode mid-route (walk->drive->walk). Runs once,
                # after every mode is built; guarded (needs >=2 modes incl. walking).
                if self.config.multimodal.enabled:
                    progress.update(main_task, description="Building multimodal graph...")
                    self._build_multimodal()

                # Base-map feature layers (features.*: water, land, buildings, POIs, transit,
                # places, …), extracted once from raw.* into THIS db for base maps (mapstyle).
                # Runs after modes (needs raw.*, still present until _cleanup), so only when
                # building from a PBF; on by default, options.build_features / --no-features.
                if self.config.options.build_features and self.config.source.type == "pbf":
                    progress.update(main_task, description="Building feature layers...")
                    self._build_features()

                # Persist the canonical stable-edge_id macro (callable anywhere as
                # `edge_id_hash(osm_id, source, target)`, plus the legacy `edge_id_hash_v1`), so
                # the formula travels with the db and other projects reuse one implementation.
                from duckosm.edge_id import create_edge_id_macro
                create_edge_id_macro(self.con)

                # Final cleanup
                progress.update(main_task, description="Cleaning up...")
                self._cleanup()
                progress.advance(main_task)
                
                # Generate visualization metadata in the main schema
                try:
                    self.con.execute("USE main")
                    self._generate_metadata()
                except Exception as e:
                    logger.warning(f"Failed to generate visualization_metadata: {e}")
                # Every area db carries its IANA time zone (hourly data needs local time): required,
                # so a failure here fails the build rather than leaving a db without one.
                from duckosm.utils import add_timezone, stamp_build
                logger.info(f"  Timezone: {add_timezone(self.con)}")
                stamp_build(self.con)

                # Build report (D) + roadstyle viz (E)
                if self.config.report.enabled:
                    try:
                        from duckosm.report import write_report
                        write_report(self.con, self.config, self.mode_stats,
                                     self.validation_results)
                    except Exception as e:
                        logger.warning(f"Report generation failed: {e}")
                if self.config.viz.enabled:
                    try:
                        from duckosm.viz import render_network
                        for mode in self.config.modes:
                            render_network(self.con, mode, self.config.name,
                                           self.config.viz.basemap)
                    except Exception as e:
                        logger.warning(f"Viz generation failed: {e}")

                # Final checkpoint to ensure disk persistence
                self._checkpoint()
                
        finally:
            if self.con:
                self.con.close()
            
        total_time = time.time() - total_start
        console.print(f"\n[bold green]✓ Import completed in {total_time:.2f}s[/bold green]")
        
        self._print_stats()
        
        return self.output_path
    
    def _connect(self) -> None:
        """Connect to DuckDB and load extensions."""
        logger.info("Connecting to DuckDB...")
        check_db_name(self.output_path)       # before anything is written or removed
        # Start every build from a clean database file so a rebuild can never inherit stale
        # tables from a previous run (e.g. tables a since-removed pipeline step used to write).
        # Only the OUTPUT file is removed here — a clip's source/parent db is a different file.
        wal = self.output_path.with_name(self.output_path.name + ".wal")
        for stale in (self.output_path, wal):
            if stale.exists():
                stale.unlink()
        self.con = duckdb.connect(str(self.output_path))
        self.con.execute("INSTALL spatial; LOAD spatial;")

        # Large-build tuning: spill big intermediates to disk instead of OOM, and
        # avoid buffering rows just to preserve insertion order (edge_id only needs
        # to be unique, not ordered). Critical for country-scale extracts.
        tmp_dir = self.output_path.parent / f"{self.output_path.stem}.tmp"
        self.con.execute(f"SET temp_directory = '{tmp_dir}'")
        self.con.execute("SET preserve_insertion_order = false")
        if self.config.options.memory_limit:
            self.con.execute(f"SET memory_limit = '{self.config.options.memory_limit}'")
        if self.config.options.threads:
            self.con.execute(f"SET threads = {self.config.options.threads}")
        
        # Try to load H3 extension
        try:
            self.con.execute("INSTALL h3 FROM community; LOAD h3;")
            self._h3_available = True
        except Exception:
            logger.warning("H3 extension not available, using Python fallback")
            self._h3_available = False
    
    def _load_pbf(self) -> None:
        """Load PBF file using ST_READOSM and save raw data."""
        logger.info("Loading PBF file...")
        start = time.time()
        
        self.con.execute(f"""
            CREATE OR REPLACE VIEW osm_raw AS 
            SELECT * FROM ST_READOSM('{self.pbf_path}')
        """)
        
        # Create raw schema with all OSM data
        self.con.execute("CREATE SCHEMA IF NOT EXISTS raw")
        
        # Save all nodes
        self.con.execute("""
            CREATE OR REPLACE TABLE raw.nodes AS
            SELECT 
                id AS osm_id,
                lat,
                lon,
                tags
            FROM osm_raw
            WHERE kind = 'node'
            AND lat IS NOT NULL 
            AND lon IS NOT NULL
        """)
        
        # Save all ways
        self.con.execute("""
            CREATE OR REPLACE TABLE raw.ways AS
            SELECT 
                id AS osm_id,
                tags,
                refs
            FROM osm_raw
            WHERE kind = 'way'
            AND len(refs) >= 2
        """)
        
        # Save all relations
        self.con.execute("""
            CREATE OR REPLACE TABLE raw.relations AS
            SELECT 
                id AS osm_id,
                tags,
                refs,
                ref_roles,
                ref_types
            FROM osm_raw
            WHERE kind = 'relation'
        """)
        
        self.stats['pbf_load_time'] = time.time() - start
        
        # Get raw counts
        raw_nodes = self.con.execute("SELECT COUNT(*) FROM raw.nodes").fetchone()[0]
        raw_ways = self.con.execute("SELECT COUNT(*) FROM raw.ways").fetchone()[0]
        raw_rels = self.con.execute("SELECT COUNT(*) FROM raw.relations").fetchone()[0]
        
        logger.info(f"  PBF loaded in {self.stats['pbf_load_time']:.2f}s")
        logger.info(f"  Raw: {raw_nodes:,} nodes, {raw_ways:,} ways, {raw_rels:,} relations")
    
    def _effective_boundary_path(self) -> Optional[str]:
        """Boundary file to clip against: the raw path, or a cached outward-buffered copy when
        ``boundary.buffer_m > 0``. The buffer is applied in metres in an auto-selected UTM zone
        (from the boundary centroid), so a `margin` grows the clip region uniformly on the ground —
        used to keep roadside sensors just outside an admin boundary on the network."""
        raw = self.config.effective_boundary_path
        if not raw:
            return None
        buf = getattr(self.config.boundary, "buffer_m", 0.0) or 0.0
        if buf <= 0:
            return raw
        if getattr(self, "_buffered_boundary_path", None):
            return self._buffered_boundary_path
        raw_p = Path(raw).resolve()
        lon, lat = self.con.execute(
            f"SELECT ST_X(ST_Centroid(geom)), ST_Y(ST_Centroid(geom)) "
            f"FROM ST_Read('{raw_p}') LIMIT 1").fetchone()
        from duckosm.utils import utm_epsg
        srid = utm_epsg(lon, lat).removeprefix("EPSG:")                 # UTM zone from centroid
        out = raw_p.with_name(f"{raw_p.stem}.buffer{int(buf)}m.geojson")
        self.con.execute(f"""
            COPY (
              SELECT ST_Transform(
                       ST_Buffer(ST_Transform(geom, 'EPSG:4326', 'EPSG:{srid}', always_xy := true), {buf}),
                       'EPSG:{srid}', 'EPSG:4326', always_xy := true) AS geom
              FROM ST_Read('{raw_p}')
            ) TO '{out}' (FORMAT gdal, DRIVER 'GeoJSON')
        """)
        logger.info(f"  Boundary buffered +{buf:g} m (UTM {srid}) -> {out.name}")
        self._buffered_boundary_path = str(out)
        return self._buffered_boundary_path

    def _load_boundary(self) -> None:
        """Load optional boundary GeoJSON file into database."""
        if not self.config.effective_boundary_path:
            return

        logger.info("Loading boundary GeoJSON...")
        start = time.time()

        boundary_path = Path(self._effective_boundary_path()).resolve()
        
        self.con.execute(f"""
            CREATE TABLE IF NOT EXISTS boundary AS
            SELECT * FROM ST_READ('{boundary_path}')
        """)
        
        count = self.con.execute("SELECT COUNT(*) FROM boundary").fetchone()[0]
        elapsed = time.time() - start
        logger.info(f"  Boundary loaded: {count} features in {elapsed:.2f}s")
    
    def _build_boundary_cells(self) -> None:
        """Generate the main.boundary_cells H3 grid from the boundary (optional)."""
        from duckosm.processors.boundary_cells import BoundaryCellsBuilder
        resolutions = (self.config.options.boundary_cell_resolutions
                       or [self.config.options.h3_resolution])
        BoundaryCellsBuilder(
            self.con, Path(self._effective_boundary_path()).resolve(), resolutions
        ).run()

    # ---- source.type: pbf — in-pipeline osmium clip (A1) -----------------------------
    def _clip_pbf(self) -> None:
        """Pre-clip the source PBF to the boundary with osmium so `boundary` actually
        constrains the graph. Idempotent (caches to pbf/<name>.osm.pbf). Falls back to
        the original PBF if osmium is unavailable or the clip fails."""
        osmium = shutil.which("osmium")
        if not osmium:
            logger.warning("osmium not found — skipping in-pipeline clip; using PBF as-is")
            return
        boundary = Path(self._effective_boundary_path()).resolve()
        cache_dir = Path("pbf")
        cache_dir.mkdir(parents=True, exist_ok=True)
        # Extract strategy, in order of completeness:
        #  - complete_ways : every way with a node in the region kept in FULL — all its nodes
        #    incl. shared intersection junctions (connected graph, intact way geometry). But it
        #    does NOT complete multipolygon relations: a river/landcover/coastline polygon whose
        #    member ways extend past the boundary loses those members and fails to close (its
        #    ST_BuildArea comes out empty and the polygon is dropped).
        #  - smart : complete_ways PLUS completing multipolygon/boundary relations (pulls in
        #    every member way), so those area features build correctly.
        # Default to `smart` when we build the features base map (needs whole multipolygons),
        # else `complete_ways` (leaner, enough for routing). Overridable via options.clip_strategy.
        strategy = self.config.options.clip_strategy or (
            "smart" if self.config.options.build_features else "complete_ways")
        # Strategy is part of the cache key: a clip made with a different strategy is NOT
        # interchangeable (complete_ways drops multipolygon members smart keeps), so it must never be
        # silently reused — that once dropped the Emajõgi river polygon on a features build. So are
        # the boundary and the source PBF: a new area or a newer download is cut again.
        clipped = pbf_cut_path(cache_dir, self.config.name, strategy, self.pbf_path, boundary).resolve()
        for old in cache_dir.glob(f"{self.config.name}.{strategy}.*osm.pbf"):
            if old.resolve() not in (clipped, self.pbf_path.resolve()):   # an outdated cut of this area
                logger.info(f"Removing outdated cut {old.name} (the boundary or the PBF changed)")
                old.unlink()
        if not clipped.exists():
            from duckosm.area import clip_pbf
            logger.info(f"Clipping {self.pbf_path.name} -> {clipped} (osmium, --strategy {strategy}) ...")
            try:
                clip_pbf(self.pbf_path, boundary, clipped, strategy)
            except RuntimeError as e:
                logger.warning(f"{e}; using PBF as-is")
                return
        self.pbf_path = clipped

    # ---- source.type: duckdb — clip an area out of a parent build (A2) ----------------
    def _attach_parent(self) -> None:
        """Attach the parent duckOSM db (read-only) as `parent`."""
        logger.info(f"Attaching parent {self.source_db} ...")
        self.con.execute(f"ATTACH '{self.source_db}' AS parent (READ_ONLY)")

    def _clip_mode(self, mode: str) -> None:
        """Clip one mode's tables out of the parent (edge_ids preserved)."""
        start = time.time()
        DuckdbClipper(self.con, mode=mode, parent_alias="parent",
                      predicate=self.config.clip.predicate).run()
        self.stats['edge_count'] = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        self.stats['node_count'] = self.con.execute("SELECT COUNT(*) FROM nodes").fetchone()[0]
        try:
            self.stats['edge_graph_count'] = self.con.execute(
                "SELECT COUNT(*) FROM edge_graph").fetchone()[0]
        except Exception:
            self.stats['edge_graph_count'] = 0
        logger.info(f"  [{mode}] clipped in {time.time() - start:.2f}s")

    # ---- ComponentFilter — drop boundary stubs / fragments (A3) -----------------------
    def _component_filter(self, mode: str) -> None:
        c = self.config.clip
        ComponentFilter(self.con, keep_largest=c.keep_largest_component,
                        min_component_edges=c.min_component_edges,
                        connectivity_rescue=c.connectivity_rescue).run()
        self.stats['edge_count'] = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        self.stats['node_count'] = self.con.execute("SELECT COUNT(*) FROM nodes").fetchone()[0]
        try:
            self.stats['edge_graph_count'] = self.con.execute(
                "SELECT COUNT(*) FROM edge_graph").fetchone()[0]
        except Exception:
            pass
        # If this mode produced an edge_id_map (merge_segments on a pbf build), prune rows
        # whose merged edge was just removed, so the table only references live edges. A
        # duckdb clip has no local edge_id_map (the unqualified name doesn't resolve to the
        # attached parent's), so the query raises and we simply skip.
        if self.config.options.merge_segments:
            live = "(SELECT edge_id FROM edges UNION ALL SELECT edge_id FROM private_edges)"
            try:
                stale = self.con.execute(
                    f"SELECT COUNT(*) FROM edge_id_map WHERE new_edge_id NOT IN {live}").fetchone()[0]
            except Exception:
                stale = 0
            if stale:
                self.con.execute(f"DELETE FROM edge_id_map WHERE new_edge_id NOT IN {live}")
                logger.info(f"  edge_id_map: pruned {stale:,} row(s) for component-dropped edges")

    # ---- validation (C) ---------------------------------------------------------------
    def _validate(self, mode: str) -> None:
        from dataclasses import replace

        from duckosm.validate import Validator
        vcfg = self.config.validation
        # "One connected network" only holds after ComponentFilter, which needs a boundary. A plain
        # extract legitimately has islands and fragments, so don't fail it on them.
        if not self._do_components and (vcfg.assert_single_component or vcfg.assert_no_stranded_named):
            logger.info("  no component clean-up ran: skipping the single-component checks")
            vcfg = replace(vcfg, assert_single_component=False, assert_no_stranded_named=False)
        self.validation_results[mode] = Validator(self.con, mode, vcfg).run()

    def _filter_roads(self, mode: str) -> None:
        """Filter to highway ways only."""
        logger.info(f"[{mode}] Filtering roads...")
        start = time.time()
        
        RoadFilter(self.con, mode=mode,
                   cycling_dismount=self.config.options.cycling_dismount).run()
        
        self.stats['road_filter_time'] = time.time() - start
        
        # Get counts
        self.stats['node_count'] = self.con.execute(
            "SELECT COUNT(*) FROM nodes"
        ).fetchone()[0]
        self.stats['way_count'] = self.con.execute(
            "SELECT COUNT(*) FROM ways"
        ).fetchone()[0]
        
        logger.info(f"  Filtered to {self.stats['node_count']:,} nodes, "
                   f"{self.stats['way_count']:,} ways in {self.stats['road_filter_time']:.2f}s")
    
    def _apply_osm_overrides(self, mode: str | None = None) -> None:
        """Patch `ways` with local corrections for known OSM errors (before edges are built)."""
        n = OsmOverrides(self.con, self.config.osm_overrides, mode=mode).run()
        self.stats['osm_overrides_applied'] = self.stats.get('osm_overrides_applied', 0) + n

    def _build_edges(self) -> None:
        """Create directed edges from ways."""
        logger.info("Building edges...")
        start = time.time()
        
        GraphBuilder(self.con).run()
        
        # Get count
        res = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()
        self.stats['edge_count'] = res[0]
        self.stats['edge_build_time'] = time.time() - start
        logger.info(f"  Created {self.stats['edge_count']:,} edges in {self.stats['edge_build_time']:.2f}s")

    def _build_global_junctions(self) -> None:
        """Build main.global_junctions (mode-agnostic road-junction set) once, before the mode loop.

        GraphSimplifier auto-detects this table and, when present, segments every mode's roads at this
        shared set so a road keeps the same edge_id across driving/walking/cycling.
        """
        GlobalJunctions(self.con).run()

    def _add_functional_type(self, mode: str) -> None:
        """Add walk_type / cycle_type to the mode's edges from OSM sub-tags (walking/cycling only)."""
        FunctionalType(self.con, mode=mode).run()

    def _split_private(self) -> None:
        """Move the mode's private roads out of `edges` into `private_edges` (same columns), so
        routing, the graph of legal turns and every export only see roads you may use; the maps
        still draw them. See docs/design/access_private.md."""
        self.con.execute("CREATE OR REPLACE TABLE private_edges AS SELECT * FROM edges WHERE access = 'private'")
        n = self.con.execute("SELECT count(*) FROM private_edges").fetchone()[0]
        if n:
            self.con.execute("DELETE FROM edges WHERE access = 'private'")
            self.con.execute("DELETE FROM nodes WHERE node_id NOT IN "
                             "(SELECT source FROM edges UNION SELECT target FROM edges)")
        logger.info(f"  {n:,} private edges moved to private_edges")

    def _mark_dismount(self) -> None:
        """Flag cycling edges that are walked, not ridden (dismount column)."""
        from duckosm.processors.dismount import DismountMarker
        DismountMarker(self.con).run()

    def _simplify_graph(self) -> None:
        """Simplify the road network graph."""
        start = time.time()

        GraphSimplifier(self.con, batches=self.config.options.simplify_batches,
                        merge_segments=self.config.options.merge_segments).run()
        
        # Update stats
        res = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()
        self.stats['edge_count'] = res[0]
        res = self.con.execute("SELECT COUNT(*) FROM nodes").fetchone()
        self.stats['node_count'] = res[0]
        
        self.stats['simplification_time'] = time.time() - start
        logger.info(f"  Graph simplified: {self.stats['node_count']:,} nodes, {self.stats['edge_count']:,} edges")

    def _connect_paths(self, mode: str) -> None:
        """Reconnect dangling cycleway/footway ends to the network (PathConnector) before the
        component filter would drop them. See docs/design/connectivity_repair.md."""
        before = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        PathConnector(self.con, snap_m=self.config.clip.connect_snap_m).run()
        after = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        self.stats['edge_count'] = after
        self.stats['node_count'] = self.con.execute("SELECT COUNT(*) FROM nodes").fetchone()[0]

    def _process_speeds(self, mode: str) -> None:
        """Process and fill missing speed limits."""
        logger.info(f"[{mode}] Processing speeds...")
        start = time.time()
        
        SpeedProcessor(self.con, mode=mode).run()
        
        self.stats['speed_time'] = time.time() - start
        logger.info(f"  Speeds processed in {self.stats['speed_time']:.2f}s")
    
    def _calculate_costs(self, mode: str) -> None:
        """Calculate travel time costs."""
        logger.info(f"[{mode}] Calculating costs...")
        start = time.time()
        
        CostCalculator(self.con).run()
        
        self.stats['cost_time'] = time.time() - start
        logger.info(f"  Costs calculated in {self.stats['cost_time']:.2f}s")
    
    def _extract_restrictions(self) -> None:
        """Extract turn restrictions."""
        logger.info("Extracting turn restrictions...")
        start = time.time()
        
        RestrictionProcessor(self.con, self.config.osm_overrides).run()   # + synthetic overrides
        
        self.stats['restriction_time'] = time.time() - start
        self.stats['restriction_count'] = self.con.execute(
            "SELECT COUNT(*) FROM turn_restrictions"
        ).fetchone()[0]
        
        logger.info(f"  Found {self.stats['restriction_count']:,} restrictions in "
                   f"{self.stats['restriction_time']:.2f}s")
    
    def _build_edge_graph(self) -> None:
        """Build edge adjacency graph."""
        logger.info("Building edge graph...")
        start = time.time()
        
        EdgeGraphBuilder(self.con).run()
        
        self.stats['edge_graph_time'] = time.time() - start
        self.stats['edge_graph_count'] = self.con.execute(
            "SELECT COUNT(*) FROM edge_graph"
        ).fetchone()[0]
        
        logger.info(f"  Created {self.stats['edge_graph_count']:,} edge pairs in "
                   f"{self.stats['edge_graph_time']:.2f}s")

    def _build_multimodal(self) -> None:
        """Build the intermodal mm.* transfer graph (walk<->drive<->cycle). Optional post-mode step;
        never aborts the build."""
        from duckosm.processors import MultimodalBuilder
        mm = self.config.multimodal
        logger.info("Building multimodal transfer graph (mm.*)...")
        start = time.time()
        try:
            stats = MultimodalBuilder(self.con, transfer_s=mm.transfer_s,
                                      transfer_costs=mm.transfer_costs).run()
            self.stats['mm_edge_count'] = stats.get('edge_count', 0)
            self.stats['mm_transfer_count'] = stats.get('transfer_count', 0)
            logger.info(f"  Multimodal graph built in {time.time() - start:.2f}s: "
                        f"{self.stats['mm_edge_count']:,} edges, "
                        f"{self.stats['mm_transfer_count']:,} transfers")
        except Exception as e:
            logger.warning(f"  multimodal build failed: {e}")

    def _build_features(self) -> None:
        """Build the features.* base-map schema (Shortbread layers) from raw.*: the base map
        (mapstyle draws it). Never aborts the build."""
        from duckosm.features import FeaturesBuilder
        logger.info("Building base-map feature layers (features.*)...")
        start = time.time()
        try:
            stats = FeaturesBuilder(self.con).run()
            self.stats['feature_layers'] = [k for k, v in stats.items() if v and v >= 0]
            logger.info(f"  Feature layers built in {time.time() - start:.2f}s: "
                        f"{len(self.stats['feature_layers'])} layers")
        except Exception as e:
            logger.warning(f"  features build failed: {e}")

    def _add_h3_indexing(self) -> None:
        """Add H3 spatial indexing."""
        logger.info("Adding H3 indexing...")
        start = time.time()
        
        H3Indexer(
            self.con,
            resolution=self.config.options.h3_resolution,
            use_extension=self._h3_available
        ).run()
        
        self.stats['h3_time'] = time.time() - start
        logger.info(f"  H3 indexing added in {self.stats['h3_time']:.2f}s")
    
    def _create_indexes(self) -> None:
        """Create database indexes."""
        logger.info("Creating indexes...")
        start = time.time()
        
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_nodes_id ON nodes(node_id)")
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_edges_id ON edges(edge_id)")
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source)")
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target)")
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_edges_geom ON edges USING RTREE (geometry)")
        self.con.execute("CREATE INDEX IF NOT EXISTS idx_nodes_geom ON nodes USING RTREE (geom)")
        
        if self.config.options.build_graph:
            self.con.execute("CREATE INDEX IF NOT EXISTS idx_eg_from ON edge_graph(from_edge)")
            self.con.execute("CREATE INDEX IF NOT EXISTS idx_eg_to ON edge_graph(to_edge)")
        
        self.stats['index_time'] = time.time() - start
        logger.info(f"  Indexes created in {self.stats['index_time']:.2f}s")
    
    def _cleanup(self) -> None:
        """Clean up temporary views (keep tables)."""
        logger.info("Cleaning up...")
        
        try:
            # Only drop the view, keep all tables for user queries
            self.con.execute("DROP VIEW IF EXISTS osm_raw")
        except Exception:
            pass
    
    def _checkpoint(self) -> None:
        """Checkpoint database to disk."""
        self.con.execute("CHECKPOINT")
        
        size_mb = self.output_path.stat().st_size / (1024 * 1024)
        self.stats['output_size_mb'] = size_mb
    
    def _generate_metadata(self) -> None:
        """
        Generate metadata for visualization (boundary, center, zoom).
        Stores result in 'visualization_metadata' table.
        """
        logger.info("Generating visualization metadata...")
        
        # Check if boundary table exists and has data
        has_boundary = False
        try:
            res = self.con.execute("SELECT COUNT(*) FROM boundary").fetchone()
            if res and res[0] > 0:
                has_boundary = True
        except Exception:
            pass
            
        if has_boundary:
            # Use actual boundary for GeoJSON, but still use Extent for center/zoom
            query = """
                CREATE OR REPLACE TABLE visualization_metadata AS
                WITH bbox AS (SELECT ST_Extent(geom) as ext FROM boundary),
                actual_geom AS (SELECT ST_Union_Agg(geom) as geom FROM boundary),
                center AS (SELECT ST_Centroid(ext) as geom FROM bbox)
                SELECT 
                    ST_AsGeoJSON(actual_geom.geom) as boundary_geojson,
                    ST_Y(center.geom) as center_lat,
                    ST_X(center.geom) as center_lon,
                    CASE 
                        WHEN (ST_XMax(bbox.ext) - ST_XMin(bbox.ext)) < 0.0001 THEN 14
                        ELSE CAST(LEAST(14, GREATEST(1, LOG2(360.0 / (ST_XMax(bbox.ext) - ST_XMin(bbox.ext))))) AS INTEGER)
                    END as initial_zoom
                FROM bbox, center, actual_geom
            """
        else:
            # Fallback: Calculate from nodes using fast MIN/MAX instead of ST_Extent
            query = """
                CREATE OR REPLACE TABLE visualization_metadata AS
                WITH bounds AS (
                    SELECT 
                        MIN(lon) as xmin, 
                        MIN(lat) as ymin, 
                        MAX(lon) as xmax, 
                        MAX(lat) as ymax 
                    FROM raw.nodes
                ),
                bbox AS (
                    SELECT ST_MakeLine([
                        ST_Point(xmin, ymin),
                        ST_Point(xmax, ymin),
                        ST_Point(xmax, ymax),
                        ST_Point(xmin, ymax),
                        ST_Point(xmin, ymin)
                    ]) as ext_line FROM bounds
                ),
                poly AS (
                    -- ST_MakePolygon turns the closed bbox ring into a polygon
                    -- (a single known ring, so no ST_Polygonize/ST_BuildArea needed).
                    SELECT ST_MakePolygon(ext_line) as ext FROM bbox
                ),
                center AS (
                    SELECT ST_Point((xmin + xmax) / 2, (ymin + ymax) / 2) as geom FROM bounds
                )
                SELECT 
                    ST_AsGeoJSON(poly.ext) as boundary_geojson,
                    ST_Y(center.geom) as center_lat,
                    ST_X(center.geom) as center_lon,
                    CASE 
                        WHEN (bounds.xmax - bounds.xmin) < 0.0001 THEN 14
                        ELSE CAST(LEAST(14, GREATEST(1, LOG2(360.0 / (bounds.xmax - bounds.xmin)))) AS INTEGER)
                    END as initial_zoom
                FROM bounds, poly, center
            """
            
        try:
            self.con.execute(query)
            meta = self.con.execute("SELECT center_lat, center_lon, initial_zoom FROM visualization_metadata").fetchone()
            if meta:
                logger.info(f"  Metadata created: Center=({meta[0]:.4f}, {meta[1]:.4f}), Zoom={meta[2]}")
            
        except Exception as e:
            logger.warning(f"Failed to generate metadata: {e}")
            # Create empty table to avoid errors later
            self.con.execute("""
                CREATE OR REPLACE TABLE visualization_metadata (
                    boundary_geojson VARCHAR,
                    center_lat DOUBLE,
                    center_lon DOUBLE,
                    initial_zoom INTEGER
                )
            """)

    def _print_stats(self) -> None:
        """Print final statistics summary."""
        from rich.table import Table
        from rich.console import Console
        
        console = Console()
        console.print("\n[bold]Import Summary[/bold]")
        
        table = Table(show_header=True, header_style="bold magenta")
        table.add_column("Mode", style="cyan")
        table.add_column("Nodes", justify="right")
        table.add_column("Edges", justify="right")
        table.add_column("Edge Pairs", justify="right")
        table.add_column("Time", justify="right")
        
        total_nodes = 0
        total_edges = 0
        
        for mode in self.config.modes:
            stats = self.mode_stats.get(mode, {})
            nodes = stats.get('node_count', 0)
            edges = stats.get('edge_count', 0)
            pairs = stats.get('edge_graph_count', 0)
            time_val = stats.get('total_time', 0)
            
            table.add_row(
                mode,
                f"{nodes:,}",
                f"{edges:,}",
                f"{pairs:,}",
                f"{time_val:.2f}s"
            )
            total_nodes += nodes
            total_edges += edges
            
        console.print(table)
        
        size_mb = self.output_path.stat().st_size / (1024 * 1024)
        console.print(f"  [bold]Output size:[/bold] {size_mb:.2f} MB")
        console.print(f"  [bold]Database Path:[/bold] {self.output_path}")
        console.print("-" * 50)
