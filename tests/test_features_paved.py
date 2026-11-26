"""The features layer ``paved``, the new rules of ``water_polygons`` / ``land`` / ``sites``, and ``duckosm features-check``
(docs/design/missing_polygons.md), on a small made-up OSM dataset.

Each test writes the data as OSM XML, turns it into a PBF with osmium, and builds it with the Rust program (it needs osmium)."""
import shutil
import subprocess
from xml.sax.saxutils import quoteattr

from cli_run import invoke
import duckdb
import pytest


pytestmark = pytest.mark.skipif(not shutil.which("osmium"), reason="osmium is needed")


class Osm:
    """The nodes, ways and relations of a made-up dataset, and the build of them."""

    def __init__(self, tmp_path):
        self.tmp, self.nodes, self.ways, self.rels = tmp_path, [], [], []

    def build(self):
        def tags(t):
            return "".join(f"<tag k={quoteattr(k)} v={quoteattr(v)}/>" for k, v in t.items())
        xml = ['<?xml version="1.0"?><osm version="0.6">']
        xml += [f'<node id="{i}" version="1" lat="{lat}" lon="{lon}"/>' for i, lat, lon in self.nodes]
        for i, t, refs in self.ways:
            xml.append(f'<way id="{i}" version="1">' + "".join(f'<nd ref="{r}"/>' for r in refs) + tags(t) + "</way>")
        for i, t, members in self.rels:
            xml.append(f'<relation id="{i}" version="1">' + "".join(
                f'<member type="way" ref="{w}" role="{role}"/>' for w, role in members) + tags(t) + "</relation>")
        xml.append("</osm>")
        (self.tmp / "toy.osm").write_text("".join(xml))
        r = subprocess.run(["osmium", "cat", "-o", str(self.tmp / "toy.osm.pbf"), "--overwrite", str(self.tmp / "toy.osm")],
                                         capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        # a config, not the flags: the layers of a build with no boundary come from options.build_features
        (self.tmp / "c.yaml").write_text(
            f"name: osm\noutput_path: {self.tmp}\nsource: {{type: pbf, pbf_path: {self.tmp / 'toy.osm.pbf'}}}\nmodes: [driving]\n"
            "options: {build_features: true, build_graph: false, bus_routes: false, levels: false, sea: false, h3_indexing: false}\n")
        out = invoke(["-c", self.tmp / "c.yaml"])
        assert out.exit_code == 0, out.output
        c = duckdb.connect(str(self.tmp / "osm.duckdb"), read_only=True)
        c.execute("LOAD spatial")
        return c


@pytest.fixture
def osm(tmp_path):
    return Osm(tmp_path)


def box(o, way_id, x, y, tags, size=0.001, closed=True):
    """A square way at (x, y) of side `size` degrees: four nodes, closed by the first node again."""
    base = way_id * 10
    pts = [(x, y), (x + size, y), (x + size, y + size), (x, y + size)]
    for i, (lon, lat) in enumerate(pts):
        o.nodes.append((base + i, lat, lon))
    o.ways.append((way_id, tags, [base, base + 1, base + 2, base + 3] + ([base] if closed else [])))


def kinds(c, table):
    return sorted(r[0] for r in c.execute(f"SELECT kind FROM features.{table}").fetchall())


def test_paved_and_the_other_new_rules(osm):
    box(osm, 1, 0.000, 0.000, {"place": "square", "name": "Place"})
    box(osm, 2, 0.002, 0.000, {"man_made": "pier"})
    box(osm, 3, 0.004, 0.000, {"highway": "pedestrian", "area": "yes"})            # a pedestrian AREA
    box(osm, 4, 0.006, 0.000, {"highway": "pedestrian"})                            # a closed pedestrian road with no area=yes: not an area
    box(osm, 5, 0.008, 0.000, {"aeroway": "helipad"})
    box(osm, 6, 0.010, 0.000, {"amenity": "marketplace"})
    box(osm, 7, 0.012, 0.000, {"area:highway": "traffic_island"})
    box(osm, 8, 0.014, 0.000, {"man_made": "quay"})
    box(osm, 9, 0.016, 0.000, {"leisure": "swimming_pool"})
    box(osm, 10, 0.018, 0.000, {"leisure": "beach_resort"})
    box(osm, 11, 0.020, 0.000, {"natural": "shrubbery"})
    box(osm, 12, 0.022, 0.000, {"tourism": "zoo"})
    # a pedestrian area mapped as a multipolygon relation, with no area=yes: its ring has no tags
    box(osm, 14, 0.026, 0.000, {"amenity": "grave_yard"})
    box(osm, 15, 0.028, 0.000, {"railway": "platform"})
    box(osm, 13, 0.024, 0.000, {})
    osm.rels.append((100, {"type": "multipolygon", "highway": "pedestrian"}, [(13, "outer")]))
    con = osm.build()
    assert kinds(con, "paved") == sorted(["square", "pier", "pedestrian", "pedestrian", "helipad", "marketplace", "traffic_island", "quay"])
    assert con.execute("SELECT count(*) FROM features.paved WHERE osm_type = 'relation'").fetchone()[0] == 1
    assert kinds(con, "water_polygons") == ["swimming_pool"]
    assert kinds(con, "land") == ["beach_resort", "grave_yard", "shrubbery"]
    assert kinds(con, "sites") == ["platform", "zoo"]
    # the closed pedestrian road with no area=yes is still a line in `streets`, and not a polygon in `paved`
    assert con.execute("SELECT count(*) FROM features.streets WHERE osm_id = 4").fetchone()[0] == 1
    assert con.execute("SELECT count(*) FROM features.paved WHERE osm_id = 4").fetchone()[0] == 0


def test_features_check_lists_the_misses_and_the_reasons(osm):
    box(osm, 1, 0.000, 0.000, {"place": "square"})                                  # in paved: not listed
    box(osm, 2, 0.002, 0.000, {"building:part": "yes", "height": "20"})              # in building_parts: not listed
    box(osm, 3, 0.004, 0.000, {"amenity": "fountain"})                               # left out on purpose: street furniture
    box(osm, 4, 0.006, 0.000, {"landuse": "salt_pond"})                              # a possible miss: no layer takes it, no reason
    box(osm, 5, 0.008, 0.000, {})                                                    # a ring of a relation that a table holds: not listed
    box(osm, 6, 0.010, 0.000, {"place": "neighbourhood"})                            # a zone
    osm.rels.append((100, {"type": "multipolygon", "landuse": "grass"}, [(5, "outer")]))
    osm.build().close()
    out = invoke(["features-check", osm.tmp / "osm.duckdb"])
    assert out.exit_code == 0, out.output
    lines = out.output.splitlines()
    line = lambda group: next(ln for ln in lines if group in ln)
    assert not any("place=square" in ln or "landuse=grass" in ln for ln in lines)         # held by a table
    assert "NOT EXPLAINED" in line("landuse=salt_pond")                                   # the one possible miss
    assert not any("building:part=yes" in ln for ln in lines)
    assert "furniture" in line("amenity=fountain")
    assert "zone" in line("place=neighbourhood")
    assert "1 group(s) of polygons in no features table, with no reason" in out.output
    # the possible misses come first
    assert lines.index(line("landuse=salt_pond")) < lines.index(line("amenity=fountain"))


def test_features_check_command(osm):
    box(osm, 1, 0.000, 0.000, {"landuse": "salt_pond"})
    box(osm, 2, 0.002, 0.000, {"amenity": "fountain"})
    osm.build().close()
    out = invoke(["features-check", osm.tmp / "osm.duckdb"])
    assert out.exit_code == 0, out.output
    assert "landuse=salt_pond" in out.output and "NOT EXPLAINED" in out.output and "street furniture" in out.output
    assert "1 group(s) of polygons in no features table, with no reason" in out.output


def test_features_check_needs_features(tmp_path):
    c = duckdb.connect(str(tmp_path / "empty.duckdb"))
    c.execute("CREATE SCHEMA raw")
    c.close()
    out = invoke(["features-check", str(tmp_path / "empty.duckdb")])
    assert out.exit_code != 0 and "features" in out.output
