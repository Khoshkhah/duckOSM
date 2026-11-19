-- Compare the tables of osm2pgsql (this database, schema public) with those of duckosm quack --osm2pgsql (schema ours,
-- the database of `ours` through postgres_fdw). psql -v ours=duckosm -v port=5433 -f compare.sql
\set ON_ERROR_STOP on
CREATE EXTENSION IF NOT EXISTS postgres_fdw;
DROP SERVER IF EXISTS ours CASCADE;
CREATE SERVER ours FOREIGN DATA WRAPPER postgres_fdw OPTIONS (dbname :'ours', port :'port');
CREATE USER MAPPING FOR CURRENT_USER SERVER ours;
DROP SCHEMA IF EXISTS ours CASCADE;
CREATE SCHEMA ours;
IMPORT FOREIGN SCHEMA public LIMIT TO (planet_osm_point, planet_osm_line, planet_osm_polygon, planet_osm_roads) FROM SERVER ours INTO ours;
-- a row as text: all columns, and the geometry normalized (the start and the direction of a ring do not matter)
CREATE OR REPLACE FUNCTION pg_temp.rows(s text, t text) RETURNS TABLE (osm_id bigint, r text, g geometry) LANGUAGE plpgsql AS $$
DECLARE cols text;
BEGIN
  SELECT string_agg(quote_ident(column_name), ',' ORDER BY ordinal_position) INTO cols FROM information_schema.columns
   WHERE table_schema = 'public' AND table_name = t AND column_name NOT IN ('osm_id', 'way');
  RETURN QUERY EXECUTE format('SELECT osm_id, row(%s)::text, way FROM %I.%I', cols, s, t);
END $$;
\echo 'table | rows osm2pgsql | rows duckOSM | rows only in osm2pgsql | rows only in duckOSM | geometry not ST_Equals | max distance (m)'
SELECT t,
  (SELECT count(*) FROM pg_temp.rows('public', t)),
  (SELECT count(*) FROM pg_temp.rows('ours', t)),
  (SELECT count(*) FROM (SELECT osm_id, r FROM pg_temp.rows('public', t) EXCEPT ALL SELECT osm_id, r FROM pg_temp.rows('ours', t)) x),
  (SELECT count(*) FROM (SELECT osm_id, r FROM pg_temp.rows('ours', t) EXCEPT ALL SELECT osm_id, r FROM pg_temp.rows('public', t)) x),
  (SELECT count(*) FROM pg_temp.rows('public', t) a WHERE NOT EXISTS (SELECT 1 FROM pg_temp.rows('ours', t) b WHERE b.osm_id = a.osm_id AND b.r = a.r AND ST_Equals(a.g, b.g))),
  (SELECT round(max(d)::numeric, 6) FROM (SELECT min(ST_HausdorffDistance(a.g, b.g)) d FROM pg_temp.rows('public', t) a JOIN pg_temp.rows('ours', t) b USING (osm_id) GROUP BY a.osm_id, a.g) x)
FROM unnest(ARRAY['planet_osm_point', 'planet_osm_line', 'planet_osm_polygon', 'planet_osm_roads']) t;
\echo 'the osm_id of each row that is not in both (at most 40)'
SELECT t, side, osm_id FROM (
  SELECT t, 'only osm2pgsql' side, osm_id FROM unnest(ARRAY['planet_osm_point', 'planet_osm_line', 'planet_osm_polygon', 'planet_osm_roads']) t,
    LATERAL (SELECT osm_id, r FROM pg_temp.rows('public', t) EXCEPT ALL SELECT osm_id, r FROM pg_temp.rows('ours', t)) x
  UNION ALL
  SELECT t, 'only duckOSM', osm_id FROM unnest(ARRAY['planet_osm_point', 'planet_osm_line', 'planet_osm_polygon', 'planet_osm_roads']) t,
    LATERAL (SELECT osm_id, r FROM pg_temp.rows('ours', t) EXCEPT ALL SELECT osm_id, r FROM pg_temp.rows('public', t)) x
) d ORDER BY osm_id, t, side LIMIT 40;
