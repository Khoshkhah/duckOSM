    
    def _generate_metadata(self) -> None:
        """
        Generate metadata for visualization (boundary, center, zoom).
        Stores result in 'visualization_metadata' table.
        """
        logger.info("Generating visualization metadata...")
        start = time.time()
        
        # Check if boundary table exists and has data
        has_boundary = False
        try:
            res = self.con.execute("SELECT COUNT(*) FROM boundary").fetchone()
            if res and res[0] > 0:
                has_boundary = True
        except Exception:
            pass
            
        bbox_query = ""
        if has_boundary:
            # Use provided boundary
            bbox_query = "SELECT ST_Extent(geom) FROM boundary"
        else:
            # Calculate from nodes
            bbox_query = "SELECT ST_Extent(ST_Point(lon, lat)) FROM raw.nodes"
            
        # Get bounding box (xmin, ymin, xmax, ymax)
        try:
            bbox = self.con.execute(bbox_query).fetchone()[0]
            # DuckDB ST_Extent returns a bbox struct or geometry? 
            # Actually ST_Extent usually returns a geometry (polygon of the bbox).
            # Let's verify DuckDB spatial returns.
            # ST_Extent returns a BOX_2D usually, or a Geometry. 
            # In DuckDB spatial, ST_Extent returns a GEOMETRY (Polygon).
            
            # We want simple values. ST_XMin, ST_YMin, etc. work on Geometry.
            self.con.execute(f"""
                CREATE OR REPLACE TABLE visualization_metadata AS
                WITH bbox AS ({bbox_query}),
                center AS (SELECT ST_Centroid(ST_Extent) as geom FROM bbox)
                SELECT 
                    ST_AsGeoJSON(bbox.ST_Extent) as boundary_geojson,
                    ST_Y(center.geom) as center_lat,
                    ST_X(center.geom) as center_lon,
                    -- Zoom heuristic: log2(360 / width)
                    -- Width in degrees. Handle invalid width.
                    CASE 
                        WHEN (ST_XMax(bbox.ST_Extent) - ST_XMin(bbox.ST_Extent)) < 0.0001 THEN 14
                        ELSE LEAST(14, GREATEST(1, CAST(LOG2(360.0 / (ST_XMax(bbox.ST_Extent) - ST_XMin(bbox.ST_Extent))) AS INTEGER)))
                    END as initial_zoom
                FROM bbox, center
            """)
            
            meta = self.con.execute("SELECT center_lat, center_lon, initial_zoom FROM visualization_metadata").fetchone()
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
