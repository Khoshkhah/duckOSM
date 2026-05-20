"""
H3 indexer processor - adds H3 spatial indexing.
"""

from duckosm.processors.base import BaseProcessor


class H3Indexer(BaseProcessor):
    """
    Add H3 spatial indexing to nodes and edges.
    
    Adds:
        - h3_cell column to nodes
        - source_h3/target_h3 columns to edges
    """
    
    def __init__(self, con, resolution: int = 8, use_extension: bool = False):
        """
        Initialize H3 indexer.
        
        Args:
            con: DuckDB connection
            resolution: H3 resolution (0-15)
            use_extension: Use DuckDB h3 extension if available
        """
        super().__init__(con)
        self.resolution = resolution
        self.use_extension = use_extension
    
    def run(self) -> None:
        """Add H3 indexing."""
        # Always register UDFs for LCA logic
        self._register_udfs()
        
        if self.use_extension:
            self._add_h3_with_extension()
        else:
            self._add_h3_with_python()
            
    def _register_udfs(self) -> None:
        """Register H3 UDFs."""
        from duckosm.utils import h3_calc
        
        # Helper for lat/lon (only needed for python fallback, but harmless)
        def h3_index(lat: float, lon: float) -> int:
            import h3
            if lat is None or lon is None:
                return None
            try:
                cell = h3.latlng_to_cell(lat, lon, self.resolution)
                return int(cell, 16)
            except Exception:
                return None
        
        try:
            self.con.create_function("py_h3_index", h3_index, [float, float], int)
        except Exception:
            pass # Might already exist
            
        try:
            self.con.create_function("h3_lca", h3_calc.find_lca, ["BIGINT", "BIGINT"], "BIGINT")
        except Exception:
            pass

        try:
            self.con.create_function("h3_resolution", h3_calc.get_resolution, ["BIGINT"], int)
        except Exception:
            pass

    def _add_h3_with_extension(self) -> None:
        """Add H3 using DuckDB extension."""
        self.execute(f"""
            ALTER TABLE nodes ADD COLUMN IF NOT EXISTS h3_cell BIGINT;
            UPDATE nodes SET h3_cell = h3_latlng_to_cell(ST_Y(geom), ST_X(geom), {self.resolution});
        """)
        
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS from_cell BIGINT")
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS to_cell BIGINT")
        
        self.execute("""
            UPDATE edges SET 
                from_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.source),
                to_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.target)
        """)
        
        # Add LCA Resolution
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS lca_res TINYINT")
        self.execute("UPDATE edges SET lca_res = h3_resolution(h3_lca(from_cell, to_cell))")
    
    def _add_h3_with_python(self) -> None:
        """Add H3 using Python fallback."""
        # 1. Add H3 Index to Nodes
        self.execute("""
            ALTER TABLE nodes ADD COLUMN IF NOT EXISTS h3_cell BIGINT;
            UPDATE nodes SET h3_cell = py_h3_index(ST_Y(geom), ST_X(geom));
        """)
        
        # 2. Add H3 Indices and LCA Resolution to Edges
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS from_cell BIGINT")
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS to_cell BIGINT")
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS lca_res TINYINT")
        
        # First update from/to cells using nodes
        self.execute("""
            UPDATE edges SET 
                from_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.source),
                to_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.target)
        """)
        
        # Then calculate LCA resolution
        self.execute("""
            UPDATE edges SET
                lca_res = h3_resolution(h3_lca(from_cell, to_cell))
        """)
