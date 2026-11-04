-- duckOSM 0.6.1 (2044c4f): the base-map layers (features.*) from raw.*, recorded from FeaturesBuilder by
-- tools/make_features_sql.py. Do not change it by hand. It needs DuckDB's spatial extension.

CREATE SCHEMA IF NOT EXISTS raw;
CREATE OR REPLACE TABLE raw.nodes AS SELECT osm_id, lat, lon, tags FROM read_parquet('raw_nodes.parquet');
CREATE OR REPLACE TABLE raw.ways AS SELECT osm_id, tags, refs FROM read_parquet('raw_ways.parquet');
CREATE OR REPLACE TABLE raw.relations AS
    SELECT osm_id, tags, refs, ref_roles, ref_types::ENUM('node', 'way', 'relation')[] AS ref_types
    FROM read_parquet('raw_relations.parquet');

INSTALL spatial; LOAD spatial;

CREATE SCHEMA IF NOT EXISTS geom;

CREATE OR REPLACE TABLE geom.way_line AS
    SELECT osm_id, tags, (refs[1] = refs[len(refs)]) AS is_closed, ST_GeomFromWKB(line) AS geom
    FROM read_parquet('raw_ways.parquet') WHERE line IS NOT NULL;

CREATE OR REPLACE TABLE geom.rel_member_line AS
        WITH m AS (
            SELECT osm_id AS rel_id,
                   tags   AS rel_tags,
                   UNNEST(refs)      AS ref_id,
                   UNNEST(ref_types) AS ref_type,
                   UNNEST(ref_roles) AS role
            FROM raw.relations
            WHERE map_extract(tags, 'type')[1] IN ('multipolygon', 'boundary')
        )
        SELECT m.rel_id, m.rel_tags, m.role, wl.geom
        FROM m
        JOIN geom.way_line wl ON wl.osm_id = m.ref_id
        WHERE m.ref_type = 'way';

CREATE OR REPLACE TABLE geom.rel_area AS
        SELECT rel_id AS osm_id,
               any_value(rel_tags) AS tags,
               ST_BuildArea(ST_LineMerge(ST_Collect(list(geom)))) AS geom
        FROM geom.rel_member_line
        GROUP BY rel_id;

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.land AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               CASE WHEN map_extract(tags,'natural')[1] = 'wood' THEN 'forest' ELSE COALESCE(map_extract(tags,'landuse')[1], map_extract(tags,'natural')[1], map_extract(tags,'leisure')[1], map_extract(tags,'amenity')[1]) END                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND (map_extract(tags,'landuse')[1] IN ('forest','meadow','grass','farmland','farmyard','residential','commercial','industrial','retail','garages','cemetery','allotments','orchard','vineyard','quarry','recreation_ground','village_green','greenfield','brownfield','landfill','railway','plant_nursery','flowerbed','military','religious') OR map_extract(tags,'natural')[1] IN ('wood','scrub','heath','grassland','fell','sand','beach','scree','shingle','bare_rock','wetland','shrubbery') OR map_extract(tags,'leisure')[1] IN ('park','garden','golf_course','playground','miniature_golf','nature_reserve','stadium','pitch','track','dog_park','marina','ice_rink','swimming_area','beach_resort') OR map_extract(tags,'landuse')[1] = 'greenhouse_horticulture' OR map_extract(tags,'amenity')[1] = 'grave_yard')
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               CASE WHEN map_extract(tags,'natural')[1] = 'wood' THEN 'forest' ELSE COALESCE(map_extract(tags,'landuse')[1], map_extract(tags,'natural')[1], map_extract(tags,'leisure')[1], map_extract(tags,'amenity')[1]) END                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND (map_extract(tags,'landuse')[1] IN ('forest','meadow','grass','farmland','farmyard','residential','commercial','industrial','retail','garages','cemetery','allotments','orchard','vineyard','quarry','recreation_ground','village_green','greenfield','brownfield','landfill','railway','plant_nursery','flowerbed','military','religious') OR map_extract(tags,'natural')[1] IN ('wood','scrub','heath','grassland','fell','sand','beach','scree','shingle','bare_rock','wetland','shrubbery') OR map_extract(tags,'leisure')[1] IN ('park','garden','golf_course','playground','miniature_golf','nature_reserve','stadium','pitch','track','dog_park','marina','ice_rink','swimming_area','beach_resort') OR map_extract(tags,'landuse')[1] = 'greenhouse_horticulture' OR map_extract(tags,'amenity')[1] = 'grave_yard');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.water_polygons AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               CASE WHEN map_extract(tags,'natural')[1] = 'glacier' THEN 'glacier' WHEN map_extract(tags,'landuse')[1] = 'reservoir' THEN 'reservoir' WHEN map_extract(tags,'landuse')[1] = 'basin' THEN 'basin' WHEN map_extract(tags,'waterway')[1] = 'riverbank' THEN 'river' WHEN map_extract(tags,'leisure')[1] = 'swimming_pool' THEN 'swimming_pool' WHEN map_extract(tags,'water')[1] IN ('river','canal','reservoir','basin','dock') THEN map_extract(tags,'water')[1] ELSE 'water' END                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND (map_extract(tags,'natural')[1] IN ('water','glacier') OR map_extract(tags,'waterway')[1] = 'riverbank' OR map_extract(tags,'landuse')[1] IN ('reservoir','basin') OR map_extract(tags,'water')[1] IS NOT NULL OR map_extract(tags,'leisure')[1] = 'swimming_pool')
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               CASE WHEN map_extract(tags,'natural')[1] = 'glacier' THEN 'glacier' WHEN map_extract(tags,'landuse')[1] = 'reservoir' THEN 'reservoir' WHEN map_extract(tags,'landuse')[1] = 'basin' THEN 'basin' WHEN map_extract(tags,'waterway')[1] = 'riverbank' THEN 'river' WHEN map_extract(tags,'leisure')[1] = 'swimming_pool' THEN 'swimming_pool' WHEN map_extract(tags,'water')[1] IN ('river','canal','reservoir','basin','dock') THEN map_extract(tags,'water')[1] ELSE 'water' END                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND (map_extract(tags,'natural')[1] IN ('water','glacier') OR map_extract(tags,'waterway')[1] = 'riverbank' OR map_extract(tags,'landuse')[1] IN ('reservoir','basin') OR map_extract(tags,'water')[1] IS NOT NULL OR map_extract(tags,'leisure')[1] = 'swimming_pool');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.water_lines AS
        SELECT osm_id,
               'way'                          AS osm_type,
               CASE WHEN map_extract(tags,'waterway')[1] = 'drain' THEN 'ditch' ELSE map_extract(tags,'waterway')[1] END                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               geom
        FROM geom.way_line
        WHERE (map_extract(tags,'waterway')[1] IN ('river','stream','canal','ditch','drain'));

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.paved AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               CASE WHEN map_extract(tags,'place')[1] = 'square' THEN 'square' WHEN map_extract(tags,'area:highway')[1] IS NOT NULL THEN map_extract(tags,'area:highway')[1] WHEN map_extract(tags,'man_made')[1] IN ('pier','breakwater','groyne','quay') THEN map_extract(tags,'man_made')[1] WHEN map_extract(tags,'amenity')[1] = 'marketplace' THEN 'marketplace' WHEN map_extract(tags,'aeroway')[1] IN ('helipad','heliport') THEN map_extract(tags,'aeroway')[1] ELSE map_extract(tags,'highway')[1] END                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND (map_extract(tags,'place')[1] = 'square' OR map_extract(tags,'area:highway')[1] IS NOT NULL OR (map_extract(tags,'highway')[1] IN ('pedestrian','footway','service','living_street','steps','path','cycleway') AND (map_extract(tags,'area')[1] = 'yes' OR map_extract(tags,'type')[1] = 'multipolygon')) OR map_extract(tags,'man_made')[1] IN ('pier','breakwater','groyne','quay') OR map_extract(tags,'aeroway')[1] IN ('helipad','heliport') OR map_extract(tags,'amenity')[1] = 'marketplace')
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               CASE WHEN map_extract(tags,'place')[1] = 'square' THEN 'square' WHEN map_extract(tags,'area:highway')[1] IS NOT NULL THEN map_extract(tags,'area:highway')[1] WHEN map_extract(tags,'man_made')[1] IN ('pier','breakwater','groyne','quay') THEN map_extract(tags,'man_made')[1] WHEN map_extract(tags,'amenity')[1] = 'marketplace' THEN 'marketplace' WHEN map_extract(tags,'aeroway')[1] IN ('helipad','heliport') THEN map_extract(tags,'aeroway')[1] ELSE map_extract(tags,'highway')[1] END                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND (map_extract(tags,'place')[1] = 'square' OR map_extract(tags,'area:highway')[1] IS NOT NULL OR (map_extract(tags,'highway')[1] IN ('pedestrian','footway','service','living_street','steps','path','cycleway') AND (map_extract(tags,'area')[1] = 'yes' OR map_extract(tags,'type')[1] = 'multipolygon')) OR map_extract(tags,'man_made')[1] IN ('pier','breakwater','groyne','quay') OR map_extract(tags,'aeroway')[1] IN ('helipad','heliport') OR map_extract(tags,'amenity')[1] = 'marketplace');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.sites AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               CASE WHEN map_extract(tags,'landuse')[1] = 'construction' THEN 'construction' WHEN map_extract(tags,'military')[1] = 'danger_area' THEN 'danger_area' WHEN map_extract(tags,'man_made')[1] = 'bridge' THEN 'bridge' WHEN map_extract(tags,'leisure')[1] = 'sports_centre' THEN 'sports_centre' WHEN map_extract(tags,'tourism')[1] = 'zoo' THEN 'zoo' WHEN map_extract(tags,'railway')[1] = 'platform' THEN 'platform' WHEN map_extract(tags,'amenity')[1] IS NOT NULL THEN map_extract(tags,'amenity')[1] ELSE map_extract(tags,'public_transport')[1] END                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND (map_extract(tags,'amenity')[1] IN ('parking','bicycle_parking','bus_station','university','college','school','kindergarten','hospital','prison') OR map_extract(tags,'public_transport')[1] IN ('platform','station') OR map_extract(tags,'leisure')[1] = 'sports_centre' OR map_extract(tags,'landuse')[1] = 'construction' OR map_extract(tags,'military')[1] = 'danger_area' OR map_extract(tags,'man_made')[1] = 'bridge' OR map_extract(tags,'tourism')[1] = 'zoo' OR map_extract(tags,'railway')[1] = 'platform')
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               CASE WHEN map_extract(tags,'landuse')[1] = 'construction' THEN 'construction' WHEN map_extract(tags,'military')[1] = 'danger_area' THEN 'danger_area' WHEN map_extract(tags,'man_made')[1] = 'bridge' THEN 'bridge' WHEN map_extract(tags,'leisure')[1] = 'sports_centre' THEN 'sports_centre' WHEN map_extract(tags,'tourism')[1] = 'zoo' THEN 'zoo' WHEN map_extract(tags,'railway')[1] = 'platform' THEN 'platform' WHEN map_extract(tags,'amenity')[1] IS NOT NULL THEN map_extract(tags,'amenity')[1] ELSE map_extract(tags,'public_transport')[1] END                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND (map_extract(tags,'amenity')[1] IN ('parking','bicycle_parking','bus_station','university','college','school','kindergarten','hospital','prison') OR map_extract(tags,'public_transport')[1] IN ('platform','station') OR map_extract(tags,'leisure')[1] = 'sports_centre' OR map_extract(tags,'landuse')[1] = 'construction' OR map_extract(tags,'military')[1] = 'danger_area' OR map_extract(tags,'man_made')[1] = 'bridge' OR map_extract(tags,'tourism')[1] = 'zoo' OR map_extract(tags,'railway')[1] = 'platform');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.buildings AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               map_extract(tags,'building')[1]                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND (map_extract(tags,'building')[1] IS NOT NULL AND map_extract(tags,'building')[1] <> 'no')
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               map_extract(tags,'building')[1]                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND (map_extract(tags,'building')[1] IS NOT NULL AND map_extract(tags,'building')[1] <> 'no');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.streets AS
        SELECT osm_id,
               'way'                          AS osm_type,
               COALESCE(map_extract(tags,'highway')[1], map_extract(tags,'railway')[1])                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               geom
        FROM geom.way_line
        WHERE (map_extract(tags,'highway')[1] IS NOT NULL OR map_extract(tags,'railway')[1] IN ('rail','narrow_gauge','tram','light_rail','funicular','subway','monorail'));

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.public_transport AS
        SELECT osm_id,
               'node'                         AS osm_type,
               CASE WHEN map_extract(tags,'highway')[1] = 'bus_stop' THEN 'bus_stop' WHEN map_extract(tags,'aerialway')[1] = 'station' THEN 'aerialway_station' ELSE COALESCE(map_extract(tags,'railway')[1], map_extract(tags,'amenity')[1], map_extract(tags,'aeroway')[1]) END                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               ST_Point(lon, lat)             AS geom
        FROM raw.nodes
        WHERE (map_extract(tags,'highway')[1] = 'bus_stop' OR map_extract(tags,'railway')[1] IN ('station','halt','tram_stop') OR map_extract(tags,'amenity')[1] IN ('bus_station','ferry_terminal') OR map_extract(tags,'aeroway')[1] IN ('aerodrome','helipad') OR map_extract(tags,'aerialway')[1] = 'station');

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.pois AS
        SELECT osm_id,
               'node'                         AS osm_type,
               COALESCE(map_extract(tags,'amenity')[1], map_extract(tags,'shop')[1], map_extract(tags,'tourism')[1], map_extract(tags,'office')[1], map_extract(tags,'leisure')[1], map_extract(tags,'man_made')[1])                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               ST_Point(lon, lat)             AS geom
        FROM raw.nodes
        WHERE ((map_extract(tags,'amenity')[1] IS NOT NULL AND map_extract(tags,'amenity')[1] NOT IN ('parking','bicycle_parking','bus_station','ferry_terminal')) OR map_extract(tags,'shop')[1] IS NOT NULL OR map_extract(tags,'tourism')[1] IS NOT NULL OR map_extract(tags,'office')[1] IS NOT NULL OR (map_extract(tags,'leisure')[1] IS NOT NULL AND map_extract(tags,'leisure')[1] NOT IN ('park','garden','sports_centre','golf_course','playground','nature_reserve','miniature_golf','stadium','pitch','track','dog_park','marina','ice_rink','swimming_area')) OR map_extract(tags,'man_made')[1] IS NOT NULL);

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.traffic AS
        SELECT osm_id,
               'node'                         AS osm_type,
               map_extract(tags,'highway')[1]                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               ST_Point(lon, lat)             AS geom
        FROM raw.nodes
        WHERE (map_extract(tags,'highway')[1] IN ('traffic_signals','crossing'));

ALTER TABLE features.traffic ADD COLUMN IF NOT EXISTS bearing DOUBLE;

UPDATE features.traffic SET bearing = b.bearing
            FROM (
                WITH hw AS (   -- ROAD ways only (skip the footway/cycleway/path the crossing runs on)
                    SELECT refs FROM raw.ways
                    WHERE map_extract(tags,'highway')[1] IS NOT NULL
                      AND map_extract(tags,'highway')[1] NOT IN
                          ('footway','path','steps','corridor','bridleway','cycleway','pedestrian',
                           'construction','proposed')
                ),
                pos AS (
                    SELECT p.osm_id AS node_id, h.refs AS refs, list_position(h.refs, p.osm_id) AS pp
                    FROM features.traffic p JOIN hw h ON list_contains(h.refs, p.osm_id)
                    WHERE p.kind = 'crossing'
                    QUALIFY row_number() OVER (PARTITION BY p.osm_id ORDER BY len(h.refs) DESC) = 1
                ),
                nb AS (
                    SELECT node_id, refs[greatest(pp-1, 1)] AS a, refs[least(pp+1, len(refs))] AS b
                    FROM pos
                )
                SELECT nb.node_id,
                       degrees(atan2((n2.lon-n1.lon)*cos(radians((n1.lat+n2.lat)/2)),
                                     (n2.lat-n1.lat))) AS bearing
                FROM nb JOIN raw.nodes n1 ON n1.osm_id = nb.a
                        JOIN raw.nodes n2 ON n2.osm_id = nb.b
                WHERE nb.a <> nb.b
            ) b
            WHERE features.traffic.osm_id = b.node_id;

CREATE SCHEMA IF NOT EXISTS features;

CREATE OR REPLACE TABLE features.place_labels AS
        SELECT osm_id,
               'node'                         AS osm_type,
               map_extract(tags,'place')[1]                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               ST_Point(lon, lat)             AS geom
        FROM raw.nodes
        WHERE (map_extract(tags,'place')[1] IN ('city','town','village','hamlet','suburb','quarter','neighbourhood','isolated_dwelling','farm','island','locality'));

DROP TABLE IF EXISTS geom.way_pt;

DROP TABLE IF EXISTS geom.way_line;

DROP TABLE IF EXISTS geom.rel_member_line;

DROP TABLE IF EXISTS geom.rel_area;

DROP SCHEMA IF EXISTS geom;
