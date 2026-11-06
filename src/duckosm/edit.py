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
        from duckosm.config import Config
        from duckosm.importer import DuckOSM

        fixes, db = str(Path(fixes).resolve()), str(Path(db).resolve())
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as f:
            f.write(config)
        try:
            os.chdir(folder)        # ponytail: the config's relative paths are for the folder of the build; the process does nothing else
            cfg = Config.from_yaml(f.name)
        finally:
            os.unlink(f.name)
        cfg.osm_overrides, cfg.output_path = fixes, db
        t0 = time.time()
        DuckOSM(cfg).run()
        return f"{Path(db).name} in {time.time() - t0:.0f} s, with {Path(fixes).name}"


def serve(db, fixes=None, port: int = 8790):
    """Serve the page of ``db`` on 127.0.0.1:``port`` until Ctrl+C. ``fixes``: the fixes file (default: osm_fixes.yaml beside ``db``)."""
    from duckosm import _rust

    db = Path(db).resolve()
    fixes = Path(fixes).resolve() if fixes else db.with_name("osm_fixes.yaml")
    url = _rust.edit(str(db), str(fixes), port, _Db(db))
    print(f"duckosm edit: {url}  (the fixes file: {fixes}; Ctrl+C stops)", flush=True)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
