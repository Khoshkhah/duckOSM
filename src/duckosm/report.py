"""
Build report → reports/<name>_<ts>.{md,html}.

Summarises a build: source/boundary provenance, per-mode network stats, the per-mode
highway breakdown, the component clean-up, and the validation results. Self-contained
markdown + a minimal HTML render (no network dependency).
"""
import logging
from datetime import datetime
from pathlib import Path

logger = logging.getLogger("duckosm")


def write_report(con, config, mode_stats, validation_results, out_dir="reports", ts=None):
    ts = ts or datetime.now().strftime("%Y%m%d_%H%M%S")
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)

    src = (f"duckdb ({config.source_db})" if config.source_type == "duckdb"
           else f"pbf ({config.effective_pbf_path})")
    L = [f"# duckOSM build report — {config.name}", "", f"_{ts}_", "",
         f"- **source:** {src}",
         f"- **boundary:** {config.effective_boundary_path or '(none)'}",
         f"- **clip:** predicate={config.clip.predicate}, "
         f"keep_largest={config.clip.keep_largest_component}",
         f"- **output:** {config.get_db_path()}", ""]

    L += ["## Network", "", "| mode | nodes | edges | edge pairs |", "|---|---:|---:|---:|"]
    for mode, st in mode_stats.items():
        L.append(f"| {mode} | {st.get('node_count', 0):,} | {st.get('edge_count', 0):,} "
                 f"| {st.get('edge_graph_count', 0):,} |")

    for mode in config.modes:
        try:
            rows = con.execute(
                f"SELECT highway, count(*) n FROM {mode}.edges GROUP BY 1 ORDER BY 2 DESC"
            ).fetchall()
        except Exception:
            continue
        L += ["", f"### {mode} — by highway class", "", "| highway | edges |", "|---|---:|"]
        L += [f"| {hw} | {n:,} |" for hw, n in rows]

    if validation_results:
        L += ["", "## Validation", "", "| mode · check | result | detail |", "|---|:---:|---|"]
        for mode, results in validation_results.items():
            for check, ok, detail in results:
                L.append(f"| {mode} · {check} | {'✅' if ok else '❌'} | {detail} |")

    md = "\n".join(L) + "\n"
    md_path = out / f"{config.name}_{ts}.md"
    md_path.write_text(md, encoding="utf-8")
    html_path = out / f"{config.name}_{ts}.html"
    html_path.write_text(_md_to_html(md, config.name), encoding="utf-8")
    logger.info(f"  Report: {md_path}")
    return md_path


def _md_to_html(md: str, title: str) -> str:
    """Minimal markdown → HTML (headers, tables, bullet lists, bold). No dependencies."""
    out, in_table, in_list = [], False, False

    def close_table():
        nonlocal in_table
        if in_table:
            out.append("</tbody></table>")
            in_table = False

    def close_list():
        nonlocal in_list
        if in_list:
            out.append("</ul>")
            in_list = False

    def inline(s):
        import re
        return re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", s)

    for line in md.splitlines():
        if line.startswith("|"):
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if set("".join(cells)) <= set("-: "):           # separator row
                continue
            if not in_table:
                close_list()
                out.append("<table><thead><tr>"
                           + "".join(f"<th>{inline(c)}</th>" for c in cells)
                           + "</tr></thead><tbody>")
                in_table = True
            else:
                out.append("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in cells) + "</tr>")
            continue
        close_table()
        if line.startswith("### "):
            close_list(); out.append(f"<h3>{inline(line[4:])}</h3>")
        elif line.startswith("## "):
            close_list(); out.append(f"<h2>{inline(line[3:])}</h2>")
        elif line.startswith("# "):
            close_list(); out.append(f"<h1>{inline(line[2:])}</h1>")
        elif line.startswith("- "):
            if not in_list:
                out.append("<ul>"); in_list = True
            out.append(f"<li>{inline(line[2:])}</li>")
        elif line.startswith("_") and line.endswith("_") and len(line) > 1:
            out.append(f"<p class='sub'>{inline(line[1:-1])}</p>")
        elif line.strip():
            close_list(); out.append(f"<p>{inline(line)}</p>")
    close_table(); close_list()

    style = ("body{font:14px/1.5 system-ui,Arial,sans-serif;max-width:900px;margin:24px auto;"
             "padding:0 16px;color:#222}h1{font-size:22px}h2{margin-top:26px;border-bottom:"
             "1px solid #eee;padding-bottom:4px}.sub{color:#888}table{border-collapse:collapse;"
             "margin:8px 0}th,td{border:1px solid #ddd;padding:4px 10px;text-align:left}"
             "th{background:#f4f6fb}")
    return (f"<!doctype html><html><head><meta charset='utf-8'><title>{title}</title>"
            f"<style>{style}</style></head><body>{''.join(out)}</body></html>")
