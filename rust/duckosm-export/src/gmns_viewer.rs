//! `gmns-viz`: one self-contained HTML page to look at a GMNS db (the lanes, the meso and micro networks), the port of
//! src/duckosm/gmns_viewer.py. The page is src/duckosm/templates/gmns_viewer.html with the same JSON payload.

use crate::gis_debug::J;
use crate::Result;
use duckdb::Connection;
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("../../../src/duckosm/templates/gmns_viewer.html");
const Q: f64 = 8192.0;

fn coords(wkt: &str) -> Vec<(f64, f64)> {
    let (a, b) = (wkt.find('(').map_or(0, |i| i + 1), wkt.rfind(')').unwrap_or(wkt.len()));
    wkt[a..b]
        .split(',')
        .map(|p| {
            let mut v = p.split_whitespace().map(|x| x.parse::<f64>().unwrap_or(f64::NAN));
            (v.next().unwrap_or(f64::NAN), v.next().unwrap_or(f64::NAN))
        })
        .collect()
}

fn table(con: &Connection, schema: &str, t: &str) -> Result<bool> {
    let n: i64 = con
        .query_row("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = ?", [schema, t], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

fn query<T, F: FnMut(&duckdb::Row) -> duckdb::Result<T>>(con: &Connection, sql: &str, f: F) -> Result<Vec<T>> {
    let mut st = con.prepare(sql).map_err(|e| format!("{e}\n{sql}"))?;
    let v = st.query_map([], f).map_err(|e| e.to_string())?.collect::<std::result::Result<Vec<T>, _>>();
    v.map_err(|e| e.to_string())
}

fn s(v: &str) -> J {
    J::Str(v.to_string())
}

fn oi(v: Option<i64>) -> J {
    v.map_or(J::Null, J::Int)
}

/// The counts that `write_viewer` logs: (lanes, connectors).
pub struct Viewer {
    pub path: PathBuf,
    pub lanes: i64,
    pub conns: usize,
}

/// `build_viewer_payload`: the JSON payload of the page, from gmns_<mode> (lane) and meso_<mode> / micro_<mode> where they are.
fn payload(con: &Connection, mode: &str) -> Result<(J, i64, usize)> {
    let (g, m, mi) = (format!("gmns_{mode}"), format!("meso_{mode}"), format!("micro_{mode}"));
    if !table(con, &g, "lane")? {
        return Err(format!("no {g}.lane table — is this a GMNS db with mode '{mode}'?"));
    }
    let lanes: Vec<(i64, i64, Option<String>, Option<String>, String)> = query(
        con,
        &format!("SELECT link_id, lane_num, allowed_uses, turn, ST_AsText(geom) FROM {g}.lane WHERE geom IS NOT NULL"),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    type Sec = (String, i64, Option<i64>, Option<String>, String);
    type Con = (String, Option<String>, Option<i64>, Option<i64>, Option<i64>, String);
    let (mut secs, mut cons, mut mnodes): (Vec<Sec>, Vec<Con>, Vec<String>) = (vec![], vec![], vec![]);
    if table(con, &m, "meso_link")? {
        secs = query(
            con,
            &format!("SELECT link_id, macro_link_id, lanes, facility_type, ST_AsText(geom) FROM {m}.meso_link WHERE meso_type='normal' AND geom IS NOT NULL"),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        cons = query(
            con,
            &format!(
                "SELECT link_id, mvmt_txt_id, lanes, start_ib_lane, end_ib_lane, ST_AsText(geom) FROM {m}.meso_link WHERE meso_type='movement' AND geom IS NOT NULL"
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        mnodes = query(con, &format!("SELECT ST_AsText(geom) FROM {m}.meso_node"), |r| r.get(0))?;
    }
    let micro: Vec<(String, Option<String>, String)> = if table(con, &mi, "micro_link")? {
        query(con, &format!("SELECT cell_type, mvmt_txt_id, ST_AsText(geom) FROM {mi}.micro_link WHERE geom IS NOT NULL"), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
    } else {
        vec![]
    };
    let cnt: Vec<(Option<String>, i64)> = query(con, &format!("SELECT allowed_uses, count(*) FROM {g}.lane GROUP BY 1"), |r| Ok((r.get(0)?, r.get(1)?)))?;
    let count_of = |u: &str| cnt.iter().find(|c| c.0.as_deref() == Some(u)).map_or(0, |c| c.1);
    // a lane-rich junction to open on (a bus-lane approach first, then many lanes, then a long link)
    let focus: Option<(f64, f64)> = if table(con, &g, "movement")? {
        query(
            con,
            &format!(
                "SELECT n.x_coord, n.y_coord FROM {g}.node n JOIN {g}.link l ON l.to_node_id = n.node_id
                 WHERE n.node_id IN (SELECT node_id FROM {g}.movement)
                 ORDER BY (l.link_id IN (SELECT link_id FROM {g}.lane WHERE allowed_uses='bus')) DESC, l.lanes DESC, l.\"length\" DESC LIMIT 1"
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .into_iter()
        .next()
    } else {
        None
    };
    let allc: Vec<(f64, f64)> = lanes.iter().flat_map(|l| coords(&l.4)).collect();
    if allc.is_empty() {
        return Err(format!("{g}.lane has no geometry"));
    }
    let fold = |f: fn(f64, f64) -> f64, init: f64, k: fn(&(f64, f64)) -> f64| allc.iter().map(k).fold(init, f);
    let (minx, maxx) = (fold(f64::min, f64::INFINITY, |p| p.0), fold(f64::max, f64::NEG_INFINITY, |p| p.0));
    let (miny, maxy) = (fold(f64::min, f64::INFINITY, |p| p.1), fold(f64::max, f64::NEG_INFINITY, |p| p.1));
    let kx = ((miny + maxy) / 2.0).to_radians().cos();
    let kx = if kx == 0.0 { 1.0 } else { kx };
    // Python's max(a, b, c): the first of the largest
    let big = [(maxx - minx) * kx, maxy - miny, 1e-9].into_iter().reduce(|a, b| if b > a { b } else { a }).unwrap();
    let scale = Q / big;
    let r = |v: f64| v.round_ties_even() as i64; // Python's round()
    let qx = |x: f64| r((x - minx) * kx * scale);
    let qy = |y: f64| r((maxy - y) * scale);
    let flat = |w: &str| J::Arr(coords(w).into_iter().flat_map(|(x, y)| [J::Int(qx(x)), J::Int(qy(y))]).collect());
    let use_code = |u: Option<&str>| match u {
        Some("bus") | Some("bus,bike") => 1,
        Some("bike") => 2,
        _ => 0,
    };
    let lane_feats: Vec<J> = lanes
        .iter()
        .map(|(lid, ln, u, tr, w)| {
            J::Obj(vec![
                ("t".into(), s("lane")),
                ("u".into(), J::Int(use_code(u.as_deref()))),
                ("id".into(), J::Str(lid.to_string())),
                ("n".into(), J::Int(*ln)),
                ("tr".into(), s(tr.as_deref().unwrap_or(""))),
                ("g".into(), flat(w)),
            ])
        })
        .collect();
    let sec_feats: Vec<J> = secs
        .iter()
        .map(|(sid, mid, la, fac, w)| {
            J::Obj(vec![
                ("t".into(), s("section")),
                ("id".into(), s(sid)),
                ("m".into(), J::Str(mid.to_string())),
                ("la".into(), oi(*la)),
                ("f".into(), s(fac.as_deref().unwrap_or(""))),
                ("g".into(), flat(w)),
            ])
        })
        .collect();
    let con_feats: Vec<J> = cons
        .iter()
        .map(|(cid, mv, la, sib, eib, w)| {
            J::Obj(vec![
                ("t".into(), s("conn")),
                ("id".into(), s(cid)),
                ("mv".into(), s(mv.as_deref().unwrap_or(""))),
                ("la".into(), oi(*la)),
                ("s".into(), oi(*sib)),
                ("e".into(), oi(*eib)),
                ("g".into(), flat(w)),
            ])
        })
        .collect();
    let mnode_arr: Vec<J> = mnodes.iter().map(|w| coords(w)[0]).map(|(x, y)| J::Arr(vec![J::Int(qx(x)), J::Int(qy(y))])).collect();
    let mi_cells: Vec<J> = micro.iter().filter(|m| m.0 == "normal").map(|m| flat(&m.2)).collect();
    let mi_lchg: Vec<J> = micro.iter().filter(|m| m.0 == "lane_change").map(|m| flat(&m.2)).collect();
    let mi_conns: Vec<J> = micro
        .iter()
        .filter(|m| m.0 == "movement")
        .map(|m| J::Obj(vec![("t".into(), s("mconn")), ("mv".into(), s(m.1.as_deref().unwrap_or(""))), ("g".into(), flat(&m.2))]))
        .collect();
    let focus = focus.unwrap_or(((minx + maxx) / 2.0, (miny + maxy) / 2.0));
    let (fx, fy, half) = (qx(focus.0), qy(focus.1), r(55.0 * scale / 111320.0));
    let total: i64 = cnt.iter().map(|c| c.1).sum();
    let (nsec, ncon, ncell, nmconn) = (sec_feats.len(), con_feats.len(), mi_cells.len(), mi_conns.len());
    let p = J::Obj(vec![
        ("grid".into(), J::Obj(vec![("w".into(), J::Int(r((maxx - minx) * kx * scale))), ("h".into(), J::Int(r((maxy - miny) * scale)))])),
        ("focus".into(), J::Arr(vec![J::Int(fx - half), J::Int(fy - half), J::Int(fx + half), J::Int(fy + half)])),
        ("lanes".into(), J::Arr(lane_feats)),
        ("sections".into(), J::Arr(sec_feats)),
        ("conns".into(), J::Arr(con_feats)),
        ("mnodes".into(), J::Arr(mnode_arr)),
        ("micro_cells".into(), J::Arr(mi_cells)),
        ("micro_lchg".into(), J::Arr(mi_lchg)),
        ("micro_conns".into(), J::Arr(mi_conns)),
        ("has_meso".into(), J::Bool(nsec + ncon > 0)),
        ("has_micro".into(), J::Bool(!micro.is_empty())),
        ("mode".into(), s(mode)),
        (
            "stats".into(),
            J::Obj(vec![
                ("lanes".into(), J::Int(total)),
                ("bus".into(), J::Int(count_of("bus"))),
                ("bike".into(), J::Int(count_of("bike"))),
                ("sec".into(), J::Int(nsec as i64)),
                ("con".into(), J::Int(ncon as i64)),
                ("mcell".into(), J::Int(ncell as i64)),
                ("mconn".into(), J::Int(nmconn as i64)),
                ("file".into(), s("")),
            ]),
        ),
    ]);
    Ok((p, total, ncon))
}

/// `render_viewer_html`: the page with the payload, the file's name in the title and in `stats.file`.
fn render(mut payload: J, standalone: bool, label: &str) -> String {
    if let J::Obj(v) = &mut payload {
        if let Some((_, J::Obj(st))) = v.iter_mut().find(|x| x.0 == "stats") {
            if let Some(f) = st.iter_mut().find(|x| x.0 == "file") {
                f.1 = J::Str(label.to_string());
            }
        }
    }
    let title = if label.is_empty() { "GMNS viewer".to_string() } else { format!("GMNS viewer · {label}") };
    let content = format!("<title>{title}</title>\n") + &TEMPLATE.replace("/*__P__*/null", &payload.dump(None));
    if !standalone {
        return content;
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n</head>\n<body>\n{content}\n</body>\n</html>\n"
    )
}

/// `write_viewer`: the page of mode `mode` of `gmns_db`, written to `out`.
pub fn write_viewer(gmns_db: &Path, out: &Path, mode: &str) -> Result<Viewer> {
    let cfg = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).map_err(|e| e.to_string())?;
    let con = Connection::open_with_flags(gmns_db, cfg).map_err(|e| format!("{}: {e}", gmns_db.display()))?;
    con.execute_batch("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;").map_err(|e| e.to_string())?;
    let (p, lanes, conns) = payload(&con, mode)?;
    drop(con);
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let label = gmns_db.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    std::fs::write(out, render(p, true, &label)).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("GMNS viewer [{mode}]: {} lanes, {} connectors -> {}", crate::cli::thousands(lanes), crate::cli::thousands(conns as i64), out.display());
    Ok(Viewer { path: out.to_path_buf(), lanes, conns })
}
