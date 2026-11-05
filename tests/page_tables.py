"""The digest of the tables of a build, the same SQL for the duckosm-rust page (duckdb-wasm) and for a duckOSM db (docs/design/fixes_editor.md).

For each table: the row count, an md5 of the sorted rows and an md5 of each column. A row is its columns in name order, as text; the geometry
is left out (it comes from the node ids); a MAP is its sorted key=value pairs; a float is rounded to 6 digits. ``q(sql)`` runs a query and
gives a list of dicts. ``scripts/page_tables.py`` writes the digests of the page (tests/data/page_fixes/page_tables.json), and
``tests/test_page_fixes.py`` compares a ``duckosm build`` with them.
"""
SCHEMAS = ("driving", "walking", "cycling", "features", "main")
SKIP = {"main.visualization_metadata", "main.build_info"}


def tables(q):
    rows = q("SELECT table_schema AS s, table_name AS t FROM information_schema.tables "
             f"WHERE table_schema IN {SCHEMAS} AND table_type = 'BASE TABLE' ORDER BY 1, 2")
    return [f"{r['s']}.{r['t']}" for r in rows if f"{r['s']}.{r['t']}" not in SKIP]


def columns(q, table):
    s, t = table.split(".")
    rows = q(f"SELECT column_name AS c, data_type AS d FROM information_schema.columns WHERE table_schema = '{s}' AND table_name = '{t}'")
    return {r["c"]: r["d"] for r in rows}


def _text(c, d):
    if d.startswith(("GEOMETRY", "WKB")) or d == "BLOB":
        return None
    if d.startswith("MAP"):
        return f"array_to_string(list_sort(list_transform(map_entries(\"{c}\"), x -> x.key || '=' || coalesce(x.value::VARCHAR, ''))), ';')"
    if d in ("DOUBLE", "FLOAT", "REAL"):
        return f"round(\"{c}\", 6)::VARCHAR"
    return f"\"{c}\"::VARCHAR"


def _md5(q, table, expr):
    return q(f"SELECT md5(coalesce(string_agg(x, chr(10) ORDER BY x), '')) AS h FROM (SELECT {expr} AS x FROM {table})")[0]["h"]


def digest(q, table, names=None):
    """The digest of ``table`` on the columns ``names`` (all columns but the geometry when None)."""
    types = columns(q, table)
    cols = {c: e for c in sorted(names if names is not None else types) if (e := _text(c, types[c]))}
    row = "concat_ws('|', " + ", ".join(f"coalesce({e}, '~')" for e in cols.values()) + ")"
    n = q(f"SELECT count(*)::INTEGER AS n FROM {table}")[0]["n"]
    return {"n": n, "rows": _md5(q, table, row), "columns": {c: _md5(q, table, f"coalesce({e}, '~')") for c, e in cols.items()}}


def digests(q, names=None):
    """The digests of all tables; ``names``: {table: [columns]} to use only these columns (and only these tables)."""
    return {t: digest(q, t, names[t] if names else None) for t in tables(q) if not names or t in names}
