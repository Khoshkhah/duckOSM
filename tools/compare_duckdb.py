"""Compare two .duckdb files: the tables, the column types, all values, the views and the macros.

    python tools/compare_duckdb.py page.duckdb ref.duckdb

The check of the button "Download .duckdb" of the page (docs/browser.md): ref.duckdb is the file that make_duckdb.sql
of the same build makes (`duckdb ref.duckdb < make_duckdb.sql` in the folder of the files). The values compare as text,
a GEOMETRY as its WKB. Prints one JSON line and exits with 1 when the files differ.
"""
import json
import sys

import duckdb

c = duckdb.connect()
c.execute(f"ATTACH '{sys.argv[1]}' AS a (READ_ONLY)")
c.execute(f"ATTACH '{sys.argv[2]}' AS b (READ_ONLY)")
q = lambda sql: c.execute(sql).fetchall()


def cols(db):
    """{(schema, table): [(column, type)]} of the tables of a file"""
    return {(s, t): q(f"SELECT column_name, data_type FROM information_schema.columns WHERE table_catalog = '{db}' "
                      f"AND table_schema = '{s}' AND table_name = '{t}' ORDER BY ordinal_position")
            for s, t in q(f"SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = '{db}' "
                          "AND table_type = 'BASE TABLE'")}


views = lambda db: sorted(q(f"SELECT schema_name, view_name FROM duckdb_views() WHERE database_name = '{db}' AND NOT internal"))
macros = lambda db: sorted(q(f"SELECT schema_name, function_name, macro_definition FROM duckdb_functions() "
                             f"WHERE database_name = '{db}' AND function_type = 'macro' AND NOT internal"))

diff = []
ca, cb = cols("a"), cols("b")
for k in sorted(set(ca) | set(cb)):
    if k not in ca or k not in cb:
        diff.append(f"table {k} only in {'the first' if k in ca else 'the second'} file")
    elif ca[k] != cb[k]:
        diff.append(f"columns of {k}: {ca[k]} != {cb[k]}")
    else:
        s, t = k
        sel = ", ".join(f'hex(ST_AsWKB("{n}"))' if ty.startswith("GEOMETRY") else f'"{n}"::VARCHAR' for n, ty in ca[k])
        rows = q(f'SELECT (SELECT count(*) FROM a."{s}"."{t}"), (SELECT count(*) FROM b."{s}"."{t}")')[0]
        x, y = (q(f'SELECT count(*) FROM (SELECT {sel} FROM {d}."{s}"."{t}" EXCEPT ALL SELECT {sel} FROM {e}."{s}"."{t}")')[0][0]
                for d, e in (("a", "b"), ("b", "a")))
        if rows[0] != rows[1] or x or y:
            diff.append(f"rows of {k}: {rows[0]} and {rows[1]}, {x} and {y} rows only in one file")
if views("a") != views("b"):
    diff.append(f"views {views('a')} != {views('b')}")
if macros("a") != macros("b"):
    diff.append(f"macros {macros('a')} != {macros('b')}")
print(json.dumps({"duckdb": duckdb.__version__, "tables": len(ca), "equal": not diff, "diff": diff}))
sys.exit(1 if diff else 0)
