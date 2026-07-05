"""Thematic feature layers, mapped to the Shortbread vector-tile schema.

Each layer is a small declarative class: a ``name`` (-> ``features.<name>``), a ``geom_kind``
(area / line / point, which picks the geometry writer), a tag ``where_sql`` predicate, and a
``kind_sql`` expression producing the Shortbread ``kind`` value. Adding a layer is usually just a
predicate. ``enabled = False`` registers a not-yet-ready layer that the builder skips.

Shortbread specifics that differ from a naive per-OSM-tag split:
- **streets** merges roads AND railways into one layer (``kind`` = the highway or railway value).
- **water** splits into ``water_polygons`` (areas) + ``water_lines`` (lines).
- POIs split into ``pois`` (amenity/shop/...) and ``public_transport`` (stops/stations).
- ``amenity=parking`` and other amenity *areas* go to ``sites`` (Shortbread), not ``pois``.
- Shortbread has no barriers/power layers — out of scope (they were disabled stubs in duckmap).
- **traffic** (``highway=traffic_signals`` / ``crossing``) is a duckOSM extension beyond Shortbread:
  the base map renders these road-furniture nodes, and crossings carry a ``bearing`` (the direction
  of the road they sit on) so the marking can be rotated to lie across the road.
"""

from duckosm.features.geometry import build_area_layer, build_line_layer, build_point_layer

# tag helper
def _t(k):
    return f"map_extract(tags,'{k}')[1]"


class Layer:
    name: str = ""
    geom_kind: str = "area"       # 'area' | 'line' | 'point' -> geometry writer
    enabled: bool = True
    where_sql: str = "FALSE"
    kind_sql: str = "NULL"

    def __init__(self, con):
        self.con = con

    def build(self) -> int:
        if self.geom_kind == "area":
            return build_area_layer(self.con, self.name, self.where_sql, self.kind_sql)
        if self.geom_kind == "line":
            return build_line_layer(self.con, self.name, self.where_sql, self.kind_sql)
        if self.geom_kind == "point":
            return build_point_layer(self.con, self.name, self.where_sql, self.kind_sql)
        raise ValueError(f"unknown geom_kind: {self.geom_kind!r}")


# ---- water --------------------------------------------------------------------------------------
class WaterPolygons(Layer):
    name = "water_polygons"
    geom_kind = "area"
    where_sql = (f"{_t('natural')} IN ('water','glacier') OR {_t('waterway')} = 'riverbank' "
                 f"OR {_t('landuse')} IN ('reservoir','basin') OR {_t('water')} IS NOT NULL")
    kind_sql = (
        f"CASE WHEN {_t('natural')} = 'glacier' THEN 'glacier' "
        f"WHEN {_t('landuse')} = 'reservoir' THEN 'reservoir' "
        f"WHEN {_t('landuse')} = 'basin' THEN 'basin' "
        f"WHEN {_t('waterway')} = 'riverbank' THEN 'river' "
        f"WHEN {_t('water')} IN ('river','canal','reservoir','basin','dock') THEN {_t('water')} "
        f"ELSE 'water' END")


class WaterLines(Layer):
    name = "water_lines"
    geom_kind = "line"
    where_sql = f"{_t('waterway')} IN ('river','stream','canal','ditch','drain')"
    kind_sql = f"CASE WHEN {_t('waterway')} = 'drain' THEN 'ditch' ELSE {_t('waterway')} END"


# ---- land / sites / buildings -------------------------------------------------------------------
class Land(Layer):
    name = "land"
    geom_kind = "area"
    where_sql = (
        f"{_t('landuse')} IN ('forest','meadow','grass','farmland','farmyard','residential',"
        f"'commercial','industrial','retail','garages','cemetery','allotments','orchard',"
        f"'vineyard','quarry','recreation_ground','village_green','greenfield','brownfield',"
        f"'landfill','railway','plant_nursery','flowerbed','military','religious') "
        f"OR {_t('natural')} IN ('wood','scrub','heath','grassland','fell','sand','beach','scree',"
        f"'shingle','bare_rock','wetland') "
        f"OR {_t('leisure')} IN ('park','garden','golf_course','playground','miniature_golf',"
        f"'nature_reserve','stadium','pitch','track','dog_park','marina','ice_rink',"
        f"'swimming_area') "                              # sport/leisure AREAS render as fills in OSM-Carto (not POIs)
        f"OR {_t('landuse')} = 'greenhouse_horticulture'")
    # natural=wood -> Shortbread 'forest'; everything else passes through (values match Shortbread).
    kind_sql = (
        f"CASE WHEN {_t('natural')} = 'wood' THEN 'forest' "
        f"ELSE COALESCE({_t('landuse')}, {_t('natural')}, {_t('leisure')}) END")


class Sites(Layer):
    name = "sites"
    geom_kind = "area"
    # amenity/institutional areas + transport areas (bus_station, and public_transport platform /
    # station footprints — the node-mapped stops live in `public_transport`; the AREA versions land
    # here so a bus terminal / platform renders as a filled shape).
    where_sql = (
        f"{_t('amenity')} IN ('parking','bicycle_parking','bus_station','university','college',"
        f"'school','kindergarten','hospital','prison') "
        f"OR {_t('public_transport')} IN ('platform','station') "
        f"OR {_t('leisure')} = 'sports_centre' OR {_t('landuse')} = 'construction' "
        f"OR {_t('military')} = 'danger_area' OR {_t('man_made')} = 'bridge'")
    kind_sql = (
        f"CASE WHEN {_t('landuse')} = 'construction' THEN 'construction' "
        f"WHEN {_t('military')} = 'danger_area' THEN 'danger_area' "
        f"WHEN {_t('man_made')} = 'bridge' THEN 'bridge' "
        f"WHEN {_t('leisure')} = 'sports_centre' THEN 'sports_centre' "
        f"WHEN {_t('amenity')} IS NOT NULL THEN {_t('amenity')} "
        f"ELSE {_t('public_transport')} END")


class Buildings(Layer):
    name = "buildings"
    geom_kind = "area"
    where_sql = f"{_t('building')} IS NOT NULL AND {_t('building')} <> 'no'"
    kind_sql = f"{_t('building')}"        # Shortbread buildings has no kind; carried for convenience


# ---- streets (roads + rails, merged per Shortbread) ---------------------------------------------
_RAIL = "('rail','narrow_gauge','tram','light_rail','funicular','subway','monorail')"


class Streets(Layer):
    name = "streets"
    geom_kind = "line"
    where_sql = f"{_t('highway')} IS NOT NULL OR {_t('railway')} IN {_RAIL}"
    kind_sql = f"COALESCE({_t('highway')}, {_t('railway')})"


# ---- points: pois / public_transport / places ---------------------------------------------------
class PublicTransport(Layer):
    name = "public_transport"
    geom_kind = "point"
    where_sql = (
        f"{_t('highway')} = 'bus_stop' OR {_t('railway')} IN ('station','halt','tram_stop') "
        f"OR {_t('amenity')} IN ('bus_station','ferry_terminal') "
        f"OR {_t('aeroway')} IN ('aerodrome','helipad') OR {_t('aerialway')} = 'station'")
    kind_sql = (
        f"CASE WHEN {_t('highway')} = 'bus_stop' THEN 'bus_stop' "
        f"WHEN {_t('aerialway')} = 'station' THEN 'aerialway_station' "
        f"ELSE COALESCE({_t('railway')}, {_t('amenity')}, {_t('aeroway')}) END")


class Pois(Layer):
    name = "pois"
    geom_kind = "point"
    # amenity/shop/tourism/office/leisure/man_made POIs, minus the transport ones (-> public_transport).
    where_sql = (
        f"({_t('amenity')} IS NOT NULL AND {_t('amenity')} NOT IN ('parking','bicycle_parking',"
        f"'bus_station','ferry_terminal')) "
        f"OR {_t('shop')} IS NOT NULL OR {_t('tourism')} IS NOT NULL OR {_t('office')} IS NOT NULL "
        f"OR ({_t('leisure')} IS NOT NULL AND {_t('leisure')} NOT IN ('park','garden','sports_centre',"
        f"'golf_course','playground','nature_reserve','miniature_golf','stadium','pitch','track',"
        f"'dog_park','marina','ice_rink','swimming_area')) "
        f"OR {_t('man_made')} IS NOT NULL")
    kind_sql = (f"COALESCE({_t('amenity')}, {_t('shop')}, {_t('tourism')}, {_t('office')}, "
                f"{_t('leisure')}, {_t('man_made')})")


class Traffic(Layer):
    """Traffic-control nodes — ``highway=traffic_signals`` and ``highway=crossing`` (pedestrian
    crossings). Not a Shortbread layer, but the base map renders them, so duckOSM extracts them
    here. Crossings additionally carry a ``bearing`` (the direction of the road they sit on) so the
    crossing marking can be rotated to lie across the road."""
    name = "traffic"
    geom_kind = "point"
    where_sql = f"{_t('highway')} IN ('traffic_signals','crossing')"
    kind_sql = f"{_t('highway')}"

    def build(self) -> int:
        n = build_point_layer(self.con, self.name, self.where_sql, self.kind_sql)
        # A crossing node is a vertex of the ROAD way it crosses; bearing = direction from the
        # previous to the next node along that road (so the marking rotates to lie across it). The
        # node is also a vertex of the crossing footway/cycleway (perpendicular) — exclude those.
        self.con.execute("ALTER TABLE features.traffic ADD COLUMN IF NOT EXISTS bearing DOUBLE")
        self.con.execute("""
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
            WHERE features.traffic.osm_id = b.node_id
        """)
        return n


class PlaceLabels(Layer):
    name = "place_labels"
    geom_kind = "point"
    where_sql = (f"{_t('place')} IN ('city','town','village','hamlet','suburb','quarter',"
                 f"'neighbourhood','isolated_dwelling','farm','island','locality')")
    # capital handling (capital/state_capital) can be added later; kind = the place value for now.
    kind_sql = f"{_t('place')}"


# ---- registered but not yet implemented (enable one at a time) ----------------------------------
class Boundaries(Layer):
    """Admin boundary lines. Shortbread keeps admin_level 2 (country) & 4 (state). Boundaries live
    on relations whose member ways often lack the tag, so this needs a relation-line build
    (reuse geom.rel_member_line) — placeholder, disabled."""
    name = "boundaries"
    geom_kind = "line"
    enabled = False
    where_sql = f"{_t('boundary')} = 'administrative' AND {_t('admin_level')} IN ('2','4')"
    kind_sql = f"{_t('admin_level')}"


# Bottom-to-top render order (land/water first, labels last).
LAYERS = [
    Land,
    WaterPolygons,
    WaterLines,
    Sites,
    Buildings,
    Streets,
    Boundaries,
    PublicTransport,
    Pois,
    Traffic,
    PlaceLabels,
]

__all__ = ["Layer", "LAYERS"]
