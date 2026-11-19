"""Make tests/data/monaco_geofabrik.jsonl.gz, the golden file of `duckosm quack --geofabrik`, from Geofabrik's own files.

Download on one day: https://download.geofabrik.de/europe/monaco-latest.osm.pbf (as tests/data/monaco_geofabrik.osm.pbf)
and monaco-latest-free.gpkg.zip. Then: python tools/geofabrik_golden.py monaco.gpkg <date of the README>

Each row is [table, {column: value}, [geometry type, coordinates]] with coordinates as integers of 1e-7 degrees, as
tests/data/monaco_gdal.jsonl.gz. Only the Python standard library.
"""
import gzip
import json
import sqlite3
import struct
import sys


def wkb(b, at=0):
    """A WKB geometry as [type, coordinates]; gives the geometry and the next offset."""
    kind = struct.unpack_from('<I', b, at + 1)[0]
    at += 5
    pt = lambda at: [round(v * 1e7) for v in struct.unpack_from('<dd', b, at)]
    if kind == 1:
        return ['POINT', [pt(at)]], at + 16
    if kind == 2:
        n = struct.unpack_from('<I', b, at)[0]
        return ['LINESTRING', [pt(at + 4 + 16 * i) for i in range(n)]], at + 4 + 16 * n
    if kind == 3:
        rings, at2 = [], at + 4
        for _ in range(struct.unpack_from('<I', b, at)[0]):
            n = struct.unpack_from('<I', b, at2)[0]
            rings.append(['LINEARRING', [pt(at2 + 4 + 16 * i) for i in range(n)]])
            at2 += 4 + 16 * n
        return ['POLYGON', rings], at2
    parts, at2 = [], at + 4
    for _ in range(struct.unpack_from('<I', b, at)[0]):
        g, at2 = wkb(b, at2)
        parts.append(g)
    return ['MULTIPOLYGON', parts], at2


def main(gpkg, date):
    con = sqlite3.connect(gpkg)
    tables = [r[0] for r in con.execute("select table_name from gpkg_contents order by rowid")]
    with gzip.open('tests/data/monaco_geofabrik.jsonl.gz', 'wt', compresslevel=9) as out:
        out.write(json.dumps({'geofabrik': 'monaco-latest-free.gpkg.zip', 'date': date}) + '\n')
        for t in tables:
            cur = con.execute(f'select * from "{t}" order by fid')
            names = [d[0] for d in cur.description]
            for r in cur:
                cols = {n: v for n, v in zip(names, r) if n not in ('fid', 'geom')}
                g = r[names.index('geom')]
                flags = g[3]
                skip = 8 + {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}[(flags >> 1) & 7]
                out.write(json.dumps([t, cols, wkb(g, skip)[0]], ensure_ascii=False, separators=(',', ':')) + '\n')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
