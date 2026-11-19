#!/bin/sh
# The Monaco check of duckosm quack --osm2pgsql (docs/exports/osm2pgsql.md): osm2pgsql into the database osm, duckOSM
# into the database duckosm, then compare.sql. Needs a PostgreSQL with PostGIS on PORT (default 5433) and osm2pgsql.
# sh tests/osm2pgsql/check.sh PBF DUCKOSM [extra duckosm options]
set -e
PBF=$1 DUCKOSM=$2
shift 2
PORT=${PORT:-5433}
for d in osm duckosm; do
  dropdb -p "$PORT" --if-exists "$d"
  createdb -p "$PORT" "$d"
  psql -p "$PORT" -qd "$d" -c "CREATE EXTENSION postgis; CREATE EXTENSION hstore;"
done
osm2pgsql --hstore ${O2P_ARGS:-} -d osm -P "$PORT" "$PBF" 2>&1 | grep -E "ERROR|osm2pgsql took" || true
"$DUCKOSM" quack "$PBF" --osm2pgsql --postgis "host=localhost port=$PORT dbname=duckosm" "$@" 2>&1 | grep "quack --osm2pgsql:"
psql -p "$PORT" -d osm -qX -v ours=duckosm -v port="$PORT" -f "$(dirname "$0")/compare.sql" 2>&1 | grep -v NOTICE
