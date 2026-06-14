"""
Boundary-cells processor — H3 hexagon grid covering the boundary polygon.

Produces a `main.boundary_cells` table (h3_id, resolution, geometry) for spatial
tiling/filtering of a region. Optional and only meaningful when a boundary is
supplied. Uses the `h3` and `shapely` packages (already duckOSM dependencies).
"""
import json
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)


class BoundaryCellsBuilder(BaseProcessor):
    """Build `main.boundary_cells` from a boundary GeoJSON at given H3 resolutions."""

    def __init__(self, con, boundary_path, resolutions):
        super().__init__(con)
        self.boundary_path = str(boundary_path)
        self.resolutions = list(resolutions)

    def run(self) -> None:
        import h3
        from shapely.geometry import shape, Polygon as ShapelyPolygon, mapping
        from shapely.ops import unary_union

        with open(self.boundary_path) as f:
            gj = json.load(f)
        kind = gj.get("type")
        if kind == "FeatureCollection":
            poly = unary_union([shape(ft["geometry"]) for ft in gj["features"]])
        elif kind == "Feature":
            poly = shape(gj["geometry"])
        else:
            poly = shape(gj)

        self.execute("""
            CREATE TABLE IF NOT EXISTS main.boundary_cells (
                h3_id      VARCHAR PRIMARY KEY,
                resolution INTEGER,
                geometry   GEOMETRY
            )
        """)

        def parts(p):
            return list(p.geoms) if p.geom_type == "MultiPolygon" else [p]

        def to_latlng(coords):
            # GeoJSON is (lon, lat); h3 wants (lat, lng)
            return [(lat, lon) for lon, lat in coords]

        total = 0
        for res in self.resolutions:
            # Buffer by ~one circumradius so polygon_to_cells captures hexagons that
            # overlap the boundary, not only those whose centre is inside.
            buf_deg = h3.average_hexagon_edge_length(res, "km") / 111.0
            cells = set()
            for sp in parts(poly):
                shp = mapping(sp.buffer(buf_deg))
                outer = to_latlng(shp["coordinates"][0])
                holes = [to_latlng(r) for r in shp["coordinates"][1:]]
                cells.update(h3.polygon_to_cells(h3.LatLngPoly(outer, *holes), res))

            rows = []
            for c in cells:
                ring = [(lon, lat) for lat, lon in h3.cell_to_boundary(c)]
                if not ShapelyPolygon(ring).intersects(poly):
                    continue  # drop buffer-only cells that don't touch the real area
                ring.append(ring[0])
                wkt = "POLYGON ((" + ", ".join(f"{x} {y}" for x, y in ring) + "))"
                rows.append((c, res, wkt))

            self.execute(f"DELETE FROM main.boundary_cells WHERE resolution = {res}")
            if rows:
                # Stage WKT as a typed column, then convert — ST_GeomFromText can't
                # bind a bare prepared-statement parameter.
                self.execute(
                    "CREATE OR REPLACE TEMP TABLE _bc_stage "
                    "(h3_id VARCHAR, resolution INTEGER, wkt VARCHAR)"
                )
                self.con.executemany("INSERT INTO _bc_stage VALUES (?, ?, ?)", rows)
                self.execute(
                    "INSERT INTO main.boundary_cells "
                    "SELECT h3_id, resolution, ST_GeomFromText(wkt) FROM _bc_stage"
                )
                self.execute("DROP TABLE _bc_stage")
            total += len(rows)
            logger.info(f"  Boundary cells res {res}: {len(rows):,}")

        logger.info(f"  Boundary cells: {total:,} across resolutions {self.resolutions}")
