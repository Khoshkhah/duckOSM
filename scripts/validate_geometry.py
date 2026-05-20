import duckdb
import argparse
import sys
from rich.console import Console
from rich.table import Table

console = Console()

def validate_geometry(db_path):
    con = duckdb.connect(db_path)
    con.execute("INSTALL spatial; LOAD spatial;")
    
    # Get all mode schemas
    schemas = [r[0] for r in con.execute("SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw')").fetchall()]
    
    overall_passed = True
    
    for schema in schemas:
        console.print(f"\n[bold blue]Validating Geometry for Mode: {schema}[/bold blue]")
        
        # 1. Self-Loop Check (These can be valid circular roads like cul-de-sacs)
        self_loops = con.execute(f"SELECT count(*) FROM {schema}.edges WHERE source = target").fetchone()[0]
        if self_loops == 0:
            console.print(f"  [green]✓[/green] No self-loops found.")
        else:
            console.print(f"  [yellow]⚠[/yellow] Found {self_loops} self-loops (valid circular roads like loops/cul-de-sacs).")

        # 2. Endpoint Matching
        # Check if ST_StartPoint matches source node geometry and ST_EndPoint matches target node geometry
        mismatched_endpoints = con.execute(f"""
            SELECT count(*) 
            FROM {schema}.edges e
            JOIN {schema}.nodes ns ON e.source = ns.node_id
            JOIN {schema}.nodes nt ON e.target = nt.node_id
            WHERE ST_Distance(ST_StartPoint(e.geometry), ns.geom) > 1e-9
               OR ST_Distance(ST_EndPoint(e.geometry), nt.geom) > 1e-9
        """).fetchone()[0]
        
        if mismatched_endpoints == 0:
            console.print(f"  [green]✓[/green] All edge endpoints match their topological nodes.")
        else:
            console.print(f"  [red]✗[/red] Found {mismatched_endpoints} edges where geometry endpoints do not match node coordinates.")
            overall_passed = False

        # 3. Geometry Direction (Internal consistency)
        # Check if the geometry is degenerate (zero length)
        degenerate = con.execute(f"SELECT count(*) FROM {schema}.edges WHERE ST_Length(geometry) = 0").fetchone()[0]
        if degenerate == 0:
            console.print(f"  [green]✓[/green] No degenerate (zero-length) geometries.")
        else:
            console.print(f"  [red]✗[/red] Found {degenerate} edges with zero length.")
            overall_passed = False
            
    return overall_passed

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Validate duckOSM geometry integrity.")
    parser.add_argument("--db", default="data/output/somerset.duckdb", help="Path to DuckDB file")
    args = parser.parse_args()
    
    try:
        success = validate_geometry(args.db)
        if success:
            console.print("\n[bold green]ALL GEOMETRY CHECKS PASSED![/bold green]")
            sys.exit(0)
        else:
            console.print("\n[bold red]GEOMETRY VALIDATION FAILED![/bold red]")
            sys.exit(1)
    except Exception as e:
        console.print(f"\n[bold red]Error during validation: {e}[/bold red]")
        sys.exit(1)
