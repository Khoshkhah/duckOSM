"""
Main DuckOSM importer class.
"""

import json
import logging
import time
from pathlib import Path
from typing import Optional

import duckdb

from duckosm.config import Config
from duckosm.processors import (
    RoadFilter,
    GraphBuilder,
    SpeedProcessor,
    CostCalculator,
    RestrictionProcessor,
    H3Indexer,
    EdgeGraphBuilder,
    GraphSimplifier,
)

logger = logging.getLogger("duckosm")


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
        
        self.pbf_path = Path(config.pbf_path).resolve()
        self.output_path = config.get_db_path()
        self.output_path.parent.mkdir(parents=True, exist_ok=True)
        
        self.con: Optional[duckdb.DuckDBPyConnection] = None
        self.stats = {}
        self.mode_stats = {}
    
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
        
        console.print(f"[bold blue]duckOSM Import[/bold blue]")
        console.print(f"  PBF: {self.pbf_path}")
        console.print(f"  Output: {self.output_path}")
        console.print(f"  Modes: {', '.join(self.config.modes)}")
        console.print()
        
        # 1. Global steps (run once)
        global_steps = [
            ("connect", "Connecting to DuckDB", self._connect),
            ("load_pbf", "Loading PBF file", self._load_pbf),
        ]
        
        # Add optional boundary loading
        if self.config.boundary_path:
            global_steps.append(("load_boundary", "Loading boundary", self._load_boundary))
        
        # 2. Mode-specific steps (run for each mode)
        def get_mode_steps(mode):
            steps = [
                ("filter_roads", f"[{mode}] Filtering roads", lambda: self._filter_roads(mode)),
                ("build_edges", f"[{mode}] Building edges", self._build_edges),
            ]
            
            if self.config.options.simplify:
                steps.append(("simplify_graph", f"[{mode}] Simplifying graph", self._simplify_graph))
            
            if self.config.options.process_speeds:
                steps.append(("process_speeds", f"[{mode}] Processing speeds", lambda: self._process_speeds(mode)))
            
            if self.config.options.calculate_costs:
                steps.append(("calculate_costs", f"[{mode}] Calculating costs", lambda: self._calculate_costs(mode)))
            
            if self.config.options.extract_restrictions:
                # Turn restrictions only for driving for now
                if mode == "driving":
                    steps.append(("extract_restrictions", f"[{mode}] Extracting turn restrictions", self._extract_restrictions))
            
            if self.config.options.build_graph:
                steps.append(("build_edge_graph", f"[{mode}] Building edge graph", self._build_edge_graph))
            
            if self.config.options.h3_indexing:
                steps.append(("add_h3_indexing", f"[{mode}] Adding H3 indexing", self._add_h3_indexing))
            
            steps.append(("create_indexes", f"[{mode}] Creating indexes", self._create_indexes))
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
                
                # Final cleanup
                progress.update(main_task, description="Cleaning up...")
                self._cleanup()
                progress.advance(main_task)
                
                # Generate visualization metadata
                # Switch back to main schema so metadata is accessible globally (for Streamlit)
                try:
                    self.con.execute("USE main")
                    self._generate_metadata()
                    # Also create alias 'visualization' which Streamlit expects
                    self.con.execute("CREATE OR REPLACE TABLE visualization AS SELECT * FROM visualization_metadata")
                    logger.info("  Metadata materialized in main schema (visualization)")
                except Exception as e:
                    logger.warning(f"Failed to finalize metadata in main schema: {e}")

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
        self.con = duckdb.connect(str(self.output_path))
        self.con.execute("INSTALL spatial; LOAD spatial;")
        
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
    
    def _load_boundary(self) -> None:
        """Load optional boundary GeoJSON file into database."""
        if not self.config.boundary_path:
            return
            
        logger.info("Loading boundary GeoJSON...")
        start = time.time()
        
        boundary_path = Path(self.config.boundary_path).resolve()
        
        self.con.execute(f"""
            CREATE TABLE IF NOT EXISTS boundary AS
            SELECT * FROM ST_READ('{boundary_path}')
        """)
        
        count = self.con.execute("SELECT COUNT(*) FROM boundary").fetchone()[0]
        elapsed = time.time() - start
        logger.info(f"  Boundary loaded: {count} features in {elapsed:.2f}s")
    
    def _filter_roads(self, mode: str) -> None:
        """Filter to highway ways only."""
        logger.info(f"[{mode}] Filtering roads...")
        start = time.time()
        
        RoadFilter(self.con, mode=mode).run()
        
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

    def _simplify_graph(self) -> None:
        """Simplify the road network graph."""
        start = time.time()
        
        GraphSimplifier(self.con).run()
        
        # Update stats
        res = self.con.execute("SELECT COUNT(*) FROM edges").fetchone()
        self.stats['edge_count'] = res[0]
        res = self.con.execute("SELECT COUNT(*) FROM nodes").fetchone()
        self.stats['node_count'] = res[0]
        
        self.stats['simplification_time'] = time.time() - start
        logger.info(f"  Graph simplified: {self.stats['node_count']:,} nodes, {self.stats['edge_count']:,} edges")
    
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
        
        RestrictionProcessor(self.con).run()
        
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
                    SELECT ST_Polygonize(ext_line) as ext FROM bbox
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
