"""``duckosm studio``: the build page. The page, its server and its safety rules are Rust (rust/duckosm-edit/src/studio.rs, in ``duckosm._rust``), the
same code as ``duckosm-rust studio``. This module gives the server what the module has no DuckDB for: the build is ``duckosm build`` (the Rust engine
and the SQL steps), and the page of the result opens the file with the duckdb package of Python."""
from __future__ import annotations

import time
import webbrowser
from pathlib import Path


class _Host:
    """The methods that the Rust server calls (the trait ``Host`` of rust/duckosm-edit/src/studio.rs)."""

    def db(self, path: str):
        from duckosm.edit import _Db

        return _Db(Path(path))

    def buffer(self, wkt: str, epsg: int, metres: float) -> str:
        """The buffer of an area (``boundary.buffer_m`` of duckOSM): the MULTIPOLYGON ``wkt`` grown by ``metres`` in the UTM zone ``epsg``, as GeoJSON text."""
        import duckdb

        con = duckdb.connect()
        try:
            con.execute("INSTALL spatial; LOAD spatial;")
            return con.execute("SELECT ST_AsGeoJSON(ST_Transform(ST_Buffer(ST_Transform(ST_GeomFromText(?), 'EPSG:4326', ?, true), ?), ?, 'EPSG:4326', true))",
                               [wkt, f"EPSG:{epsg}", metres, f"EPSG:{epsg}"]).fetchone()[0]
        finally:
            con.close()

    def write(self, db: str, sql: str) -> None:
        """Run the SQL script ``sql`` (with its own BEGIN and COMMIT) on the database ``db``, open for writing: card 8 saves the levels."""
        import duckdb

        con = duckdb.connect(db)
        try:
            con.execute(sql)
        except Exception:
            con.execute("ROLLBACK")
            raise
        finally:
            con.close()

    def build(self, config: str, folder: str, db: str, sink) -> str:
        from duckosm.edit import build_area

        return build_area(config, folder, db, None, sink.step)


def serve(out_dir=".", port: int = 8792, geofabrik_index: str | None = None, open_browser: bool = True):
    """Serve the page on 127.0.0.1:``port`` until Ctrl+C. Each area is made in ``out_dir/<name>/``."""
    from duckosm import _rust

    out = Path(out_dir).resolve()
    url = _rust.studio(str(out), port, _Host(), geofabrik_index)
    print(f"duckosm studio: {url}  (the areas are in {out}; Ctrl+C stops)", flush=True)
    if open_browser:
        webbrowser.open(url)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
