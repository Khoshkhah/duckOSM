import duckdb

# Several tests only `LOAD spatial`; install it once so they don't depend on test order or on a
# machine that already has the extension cached (a fresh CI runner doesn't).
duckdb.execute("INSTALL spatial")
