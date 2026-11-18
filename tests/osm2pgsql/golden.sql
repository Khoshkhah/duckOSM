-- The golden file tests/data/monaco_osm2pgsql.jsonl.gz: the tables of osm2pgsql 2.3.1 (osm2pgsql --hstore, default style)
-- of tests/data/monaco.osm.pbf, one JSON array for each row: the table, osm_id, the columns that are not NULL, the tags,
-- and the geometry as hex WKB. psql -d osm -qAtX -f golden.sql | gzip -9 > monaco_osm2pgsql.jsonl.gz
SELECT json_build_array(t, osm_id, cols, tags, wkb)::text FROM (
  SELECT 'planet_osm_point' t, osm_id, jsonb_strip_nulls(to_jsonb(r) - 'osm_id' - 'tags' - 'way') cols, hstore_to_jsonb(tags) tags, encode(ST_AsBinary(way), 'hex') wkb FROM planet_osm_point r
  UNION ALL SELECT 'planet_osm_line', osm_id, jsonb_strip_nulls(to_jsonb(r) - 'osm_id' - 'tags' - 'way'), hstore_to_jsonb(tags), encode(ST_AsBinary(way), 'hex') FROM planet_osm_line r
  UNION ALL SELECT 'planet_osm_polygon', osm_id, jsonb_strip_nulls(to_jsonb(r) - 'osm_id' - 'tags' - 'way'), hstore_to_jsonb(tags), encode(ST_AsBinary(way), 'hex') FROM planet_osm_polygon r
  UNION ALL SELECT 'planet_osm_roads', osm_id, jsonb_strip_nulls(to_jsonb(r) - 'osm_id' - 'tags' - 'way'), hstore_to_jsonb(tags), encode(ST_AsBinary(way), 'hex') FROM planet_osm_roads r
) x ORDER BY t, osm_id, wkb;
