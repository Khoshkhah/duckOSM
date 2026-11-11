"""``duckosm edit`` (docs/design/edit_page.md): the database part of the page. The page, its server and its safety rules are Rust
(rust/duckosm-edit, in ``duckosm._rust``), the same code as ``duckosm-rust edit``. The module has no DuckDB inside, so this class gives
the server the duckdb package of Python: it opens the file read only, runs the queries, and Build again runs ``duckosm build`` again."""
from __future__ import annotations

import os
import tempfile
import time
from pathlib import Path


class _Db:
    """The methods that the Rust server calls (the trait ``Db`` of rust/duckosm-edit/src/lib.rs)."""

    def __init__(self, path: Path):
        self.path, self.con = str(path), None

    def open(self):
        import duckdb

        self.con = duckdb.connect(self.path, read_only=True)
        self.con.execute("INSTALL spatial; LOAD spatial;")

    def close(self):
        if self.con is not None:
            self.con.close()
            self.con = None

    def execute(self, sql: str):
        self.con.execute(sql)

    def texts(self, sql: str) -> list:
        return [r[0] for r in self.con.execute(sql).fetchall()]

    def build(self, config: str, folder: str, fixes: str, db: str) -> str:
        """``duckosm build`` with the config of the last build (``build_config``), in its folder, with the fixes file, into ``db``."""
        return build_area(config, folder, db, fixes)


def build_area(config: str, folder: str, db: str, fixes: str | None = None, on_step=None) -> str:
    """``duckosm build`` with the config text ``config`` in the folder ``folder`` (the config's relative paths), into ``db``, with the fixes file
    ``fixes`` when there is one. The DEM of the config's ``elevation:`` section is applied after the build (needs ``duckosm[elevation]``), and the
    build's record (``build_config``, ``build_dir`` of main.visualization_metadata) is written, so that Build again of ``duckosm edit`` can run it
    again. ``on_step(text)`` gets each step. Gives a line about the build."""
    import duckdb

    from duckosm.config import Config
    from duckosm.importer import DuckOSM

    db = str(Path(db).resolve())
    with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as f:
        f.write(config)
    try:
        os.chdir(folder)        # ponytail: the config's relative paths are for the folder of the build; the process does nothing else
        cfg = Config.from_yaml(f.name)
    finally:
        os.unlink(f.name)
    cfg.output_path = db
    if fixes:
        cfg.osm_overrides = str(Path(fixes).resolve())
    t0 = time.time()
    importer = DuckOSM(cfg)
    importer.on_step = on_step
    importer.run()
    dem = cfg.elevation.dem
    if dem:
        if on_step:
            on_step("Sampling the DEM")
        try:
            from duckosm.elevation import to_elevation, write_grid
        except ImportError as e:
            raise RuntimeError(f"the DEM step needs rasterio and pyproj: pip install 'duckosm[elevation]' ({e})")
        e = cfg.elevation
        to_elevation(db, dem=str(Path(dem).resolve()), product=e.product, vertical_datum=e.vertical_datum, license=e.license, reference=e.reference)
        write_grid(db, Path(dem).resolve())
    con = duckdb.connect(db)
    try:
        con.execute("ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS build_config VARCHAR")
        con.execute("ALTER TABLE main.visualization_metadata ADD COLUMN IF NOT EXISTS build_dir VARCHAR")
        con.execute("UPDATE main.visualization_metadata SET build_config = ?, build_dir = ?", [config, str(Path(folder).resolve())])
    finally:
        con.close()
    return f"{Path(db).name} in {time.time() - t0:.0f} s" + (f", with {Path(fixes).name}" if fixes else "")


def serve(db, fixes=None, port: int = 8790, ask: bool = False):
    """Serve the page of ``db`` on 127.0.0.1:``port`` until Ctrl+C. ``fixes``: the fixes file (default: osm_fixes.yaml beside ``db``).
    ``ask``: ``duckosm ask`` (roadstyle's data dock and Ask in place of the editor; docs/design/ask_page.md)."""
    from duckosm import _rust

    db = Path(db).resolve()
    fixes = Path(fixes).resolve() if fixes else db.with_name("osm_fixes.yaml")
    url = (_rust.ask if ask else _rust.edit)(str(db), str(fixes), port, _Db(db))
    print(f"duckosm {'ask' if ask else 'edit'}: {url}  (the fixes file: {fixes}; Ctrl+C stops)", flush=True)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
