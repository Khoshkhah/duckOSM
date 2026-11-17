//! The steps of duckOSM after the tables are in the `.duckdb` file: the costs of the multimodal transfers, the indexes,
//! the build checks (`validation:`) and the build report (`report:`). The SQL is in `duckosm_duckdb::after`.

/// What to do after the tables are written (from the config, [`crate::config::read`]).
#[derive(Clone, Debug)]
pub struct After {
    /// duckOSM `validation:` (None: `enabled: false`).
    pub validation: Option<Validation>,
    /// duckOSM `report.enabled`: write `reports/<name>_<time>.md` and `.html`.
    pub report: Option<Report>,
    /// duckOSM `multimodal.enabled`: keep `mm.transfers` and the view `mm.edges` (a config build: off by default).
    pub multimodal: bool,
    /// duckOSM `options.boundary_cells`: `main.boundary_cells` of this boundary file at these resolutions.
    pub boundary_cells: Option<(std::path::PathBuf, Vec<u8>)>,
    /// duckOSM `multimodal.transfer_s` and `transfer_costs` (`"walking->driving": 60`): the seconds of the transfers.
    pub transfer_s: f64,
    pub transfer_costs: Vec<(String, f64)>,
}

impl Default for After {
    fn default() -> Self {
        After { validation: None, report: None, multimodal: true, boundary_cells: None, transfer_s: 60.0, transfer_costs: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct Validation {
    pub fail_on_error: bool,
    pub unique_node_id: bool,
    pub way_length_conserved: bool,
    pub layer_without_structure: bool,
}

/// The lines of the report head, as the config gives them.
#[derive(Clone, Debug)]
pub struct Report {
    /// `pbf (<path>)` or `duckdb (<path>)`.
    pub source: String,
    pub boundary: Option<String>,
    pub predicate: String,
    pub output: String,
}

#[cfg(feature = "duckdb")]
pub fn run(db: &std::path::Path, name: &str, modes: &[&str], a: &After) -> Result<(), String> {
    use duckosm_duckdb::after as sql;
    if a.multimodal {
        sql::transfer_costs(db, a.transfer_s, &a.transfer_costs)?;
    } else {
        sql::drop_multimodal(db)?;
    }
    if let Some((b, rs)) = &a.boundary_cells {
        crate::place::boundary_cells(db, b, rs)?;
    }
    sql::create_indexes(db, modes)?;
    let mut results: Vec<(String, Vec<sql::Check>)> = Vec::new();
    if let Some(v) = &a.validation {
        let c = sql::Checks {
            unique_node_id: v.unique_node_id,
            way_length_conserved: v.way_length_conserved,
            layer_without_structure: v.layer_without_structure,
        };
        for m in modes {
            let r = sql::validate(db, m, c)?;
            for (check, ok, detail) in &r {
                let word = match ok { Some(true) => "OK", Some(false) => "FAIL", None => "WARN" };
                eprintln!("  validate[{m}] {check}: {word} — {detail}");
            }
            let failed: Vec<String> = r.iter().filter(|c| c.1 == Some(false)).map(|c| format!("{}: {}", c.0, c.2)).collect();
            if !failed.is_empty() && v.fail_on_error {
                return Err(format!("[{m}] {} validation check(s) failed: {}", failed.len(), failed.join("; ")));
            }
            results.push((m.to_string(), r));
        }
    }
    if let Some(r) = &a.report {
        let (counts, ts) = sql::report_counts(db, modes)?;
        let md = report_md(name, &ts, r, &counts, &results);
        std::fs::create_dir_all("reports").map_err(|e| format!("reports: {e}"))?;
        let path = format!("reports/{name}_{ts}");
        std::fs::write(format!("{path}.md"), &md).map_err(|e| format!("{path}.md: {e}"))?;
        std::fs::write(format!("{path}.html"), md_to_html(&md, name)).map_err(|e| format!("{path}.html: {e}"))?;
        eprintln!("  Report: {path}.md");
    }
    Ok(())
}

/// The report as Markdown (duckOSM `report.write_report`).
#[cfg(feature = "duckdb")]
fn report_md(
    name: &str,
    ts: &str,
    r: &Report,
    counts: &[duckosm_duckdb::after::ModeCounts],
    results: &[(String, Vec<duckosm_duckdb::after::Check>)],
) -> String {
    let n = |x: i64| {
        let s = x.abs().to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        if x < 0 { format!("-{out}") } else { out }
    };
    let mut l = vec![
        format!("# duckOSM build report — {name}"),
        String::new(),
        format!("_{ts}_"),
        String::new(),
        format!("- **source:** {}", r.source),
        format!("- **boundary:** {}", r.boundary.as_deref().unwrap_or("(none)")),
        format!("- **clip:** predicate={}", r.predicate),
        format!("- **output:** {}", r.output),
        String::new(),
        "## Network".into(),
        String::new(),
        "| mode | nodes | edges | edge pairs |".into(),
        "|---|---:|---:|---:|".into(),
    ];
    for c in counts {
        l.push(format!("| {} | {} | {} | {} |", c.mode, n(c.nodes), n(c.edges), n(c.edge_pairs)));
    }
    for c in counts {
        l.extend([String::new(), format!("### {} — by highway class", c.mode), String::new(), "| highway | edges |".into(), "|---|---:|".into()]);
        l.extend(c.highways.iter().map(|(h, k)| format!("| {h} | {} |", n(*k))));
    }
    if !results.is_empty() {
        l.extend([String::new(), "## Validation".into(), String::new(), "| mode · check | result | detail |".into(), "|---|:---:|---|".into()]);
        for (m, rs) in results {
            for (check, ok, detail) in rs {
                let mark = match ok { Some(true) => "✅", None => "⚠️", Some(false) => "❌" };
                l.push(format!("| {m} · {check} | {mark} | {detail} |"));
            }
        }
    }
    l.join("\n") + "\n"
}

#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
/// duckOSM `report._md_to_html`: headers, tables, lists and bold, with no dependency.
fn md_to_html(md: &str, title: &str) -> String {
    fn inline(s: &str) -> String {
        // **x** -> <b>x</b>
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("**") {
            match rest[i + 2..].find("**") {
                Some(j) if j > 0 => {
                    out += &rest[..i];
                    out += &format!("<b>{}</b>", &rest[i + 2..i + 2 + j]);
                    rest = &rest[i + 2 + j + 2..];
                }
                _ => break,
            }
        }
        out + rest
    }
    let (mut out, mut table, mut list) = (Vec::<String>::new(), false, false);
    for line in md.lines() {
        if line.starts_with('|') {
            let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
            if cells.concat().chars().all(|c| "-: ".contains(c)) {
                continue;
            }
            let row = |tag: &str| cells.iter().map(|c| format!("<{tag}>{}</{tag}>", inline(c))).collect::<String>();
            if !table {
                if list {
                    out.push("</ul>".into());
                    list = false;
                }
                out.push(format!("<table><thead><tr>{}</tr></thead><tbody>", row("th")));
                table = true;
            } else {
                out.push(format!("<tr>{}</tr>", row("td")));
            }
            continue;
        }
        if table {
            out.push("</tbody></table>".into());
            table = false;
        }
        let close = |out: &mut Vec<String>, list: &mut bool| {
            if *list {
                out.push("</ul>".into());
                *list = false;
            }
        };
        if let Some(t) = line.strip_prefix("### ") {
            close(&mut out, &mut list);
            out.push(format!("<h3>{}</h3>", inline(t)));
        } else if let Some(t) = line.strip_prefix("## ") {
            close(&mut out, &mut list);
            out.push(format!("<h2>{}</h2>", inline(t)));
        } else if let Some(t) = line.strip_prefix("# ") {
            close(&mut out, &mut list);
            out.push(format!("<h1>{}</h1>", inline(t)));
        } else if let Some(t) = line.strip_prefix("- ") {
            if !list {
                out.push("<ul>".into());
                list = true;
            }
            out.push(format!("<li>{}</li>", inline(t)));
        } else if line.len() > 1 && line.starts_with('_') && line.ends_with('_') {
            out.push(format!("<p class='sub'>{}</p>", inline(&line[1..line.len() - 1])));
        } else if !line.trim().is_empty() {
            close(&mut out, &mut list);
            out.push(format!("<p>{}</p>", inline(line)));
        }
    }
    if table {
        out.push("</tbody></table>".into());
    }
    if list {
        out.push("</ul>".into());
    }
    let style = "body{font:14px/1.5 system-ui,Arial,sans-serif;max-width:900px;margin:24px auto;\
padding:0 16px;color:#222}h1{font-size:22px}h2{margin-top:26px;border-bottom:\
1px solid #eee;padding-bottom:4px}.sub{color:#888}table{border-collapse:collapse;\
margin:8px 0}th,td{border:1px solid #ddd;padding:4px 10px;text-align:left}\
th{background:#f4f6fb}";
    format!(
        "<!doctype html><html><head><meta charset='utf-8'><title>{title}</title><style>{style}</style></head><body>{}</body></html>",
        out.concat()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_html_of_the_report_is_the_html_of_duckosm() {
        // the output of duckOSM's report._md_to_html for this text
        let md = "# T — x\n\n_20260101_000000_\n\n- **source:** a\n- b\n\n## N\n\n| m | n |\n|---|---:|\n| d | 1,234 |\nend\n- c\n| x |\n";
        let want = "<h1>T — x</h1><p class='sub'>20260101_000000</p><ul><li><b>source:</b> a</li><li>b</li></ul><h2>N</h2>\
<table><thead><tr><th>m</th><th>n</th></tr></thead><tbody><tr><td>d</td><td>1,234</td></tr></tbody></table><p>end</p>\
<ul><li>c</li></ul><table><thead><tr><th>x</th></tr></thead><tbody></tbody></table>";
        let html = super::md_to_html(md, "t");
        assert_eq!(&html[html.find("<body>").unwrap() + 6..html.find("</body>").unwrap()], want);
    }
}
