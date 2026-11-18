//! `admin` (duckOSM `admin.py`): add `main.admin_boundaries` (all OSM administrative levels) to a database. GDAL in DuckDB's
//! spatial extension reads the multipolygons of the PBF, as `ogr2ogr` does for duckOSM; then each boundary gets its parent.

use crate::Result;
use duckdb::Connection;
use std::path::Path;

fn err(e: duckdb::Error) -> String {
    e.to_string()
}

/// duckOSM `admin.compute_parents`: the immediate enclosing boundary of each boundary.
const PARENTS: &str = "
ALTER TABLE admin_boundaries ADD COLUMN IF NOT EXISTS parent_osm_id BIGINT;
CREATE OR REPLACE TEMP TABLE _b AS
SELECT osm_id, admin_level, geometry,
       ST_X(ST_PointOnSurface(geometry)) AS px,
       ST_Y(ST_PointOnSurface(geometry)) AS py,
       ST_XMin(geometry) xmin, ST_XMax(geometry) xmax,
       ST_YMin(geometry) ymin, ST_YMax(geometry) ymax
FROM admin_boundaries;
CREATE OR REPLACE TEMP TABLE _parent AS
WITH cand AS (
    SELECT c.osm_id AS child_id, p.osm_id AS parent_id,
           p.admin_level AS plvl, ST_Area(p.geometry) AS parea
    FROM _b c
    JOIN _b p
      ON p.osm_id <> c.osm_id
     AND (p.admin_level < c.admin_level OR c.admin_level IS NULL)
     AND c.px BETWEEN p.xmin AND p.xmax
     AND c.py BETWEEN p.ymin AND p.ymax
     AND ST_Contains(p.geometry, ST_Point(c.px, c.py))
)
SELECT child_id, parent_id FROM (
    SELECT child_id, parent_id,
           row_number() OVER (
               PARTITION BY child_id ORDER BY plvl DESC NULLS LAST, parea ASC) rn
    FROM cand
) WHERE rn = 1;
UPDATE admin_boundaries SET parent_osm_id = _parent.parent_id
FROM _parent WHERE admin_boundaries.osm_id = _parent.child_id;
DROP TABLE IF EXISTS _b;
DROP TABLE IF EXISTS _parent;
";

/// `duckosm admin --pbf PBF --db DB [--gpkg FILE]`. `gpkg`: reused when it is there, else written there.
pub fn admin(pbf: &Path, db: &Path, gpkg: Option<&Path>) -> Result<()> {
    if !pbf.exists() {
        return Err(format!("PBF not found: {}", pbf.display()));
    }
    if !db.exists() {
        return Err(format!("DB not found: {}", db.display()));
    }
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let conf = duckosm_build::place::write_osmconf()?;
    let res = (|| -> Result<()> {
        let con = Connection::open(db).map_err(|e| format!("{}: {e}", db.display()))?;
        con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(err)?;
        let source = match gpkg {
            Some(g) if g.exists() => {
                println!("Reusing existing GeoPackage: {}", g.display());
                format!("SELECT * FROM ST_Read('{}')", q(g))
            }
            Some(g) => {
                println!("Extracting admin boundaries from {} -> {} ...", pbf.display(), g.display());
                con.execute_batch(&format!(
                    // GDAL's own id column (OGC_FID) is not an attribute that the writer takes
                    "COPY (SELECT * EXCLUDE (OGC_FID) FROM ({})) TO '{}' (FORMAT gdal, DRIVER 'GPKG', LAYER_NAME 'multipolygons')",
                    duckosm_build::place::admin_multipolygons(pbf, &conf),
                    q(g)
                ))
                .map_err(|e| format!("{}: {e}", g.display()))?;
                format!("SELECT * FROM ST_Read('{}')", q(g))
            }
            None => {
                println!("Extracting admin boundaries from {} ...", pbf.display());
                duckosm_build::place::admin_multipolygons(pbf, &conf)
            }
        };
        println!("Loading admin_boundaries into {} ...", db.display());
        con.execute_batch(&format!("CREATE OR REPLACE TABLE admin_boundaries AS {}", duckosm_build::place::admin_select(&source)))
            .map_err(|e| format!("admin_boundaries: {e}"))?;
        con.execute_batch(PARENTS).map_err(|e| format!("admin_boundaries parents: {e}"))?;
        let mut st = con
            .prepare("SELECT admin_level, count(*) FROM admin_boundaries GROUP BY admin_level ORDER BY admin_level NULLS LAST")
            .map_err(err)?;
        let rows: Vec<(Option<i32>, i64)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.collect::<std::result::Result<_, _>>().map_err(err)?;
        con.execute_batch("CHECKPOINT").map_err(err)?;
        println!("admin_boundaries: {} features", rows.iter().map(|r| r.1).sum::<i64>());
        for (level, n) in rows {
            println!("  admin_level {:>4}: {n}", level.map_or("NULL".to_string(), |l| l.to_string()));
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&conf);
    res
}
