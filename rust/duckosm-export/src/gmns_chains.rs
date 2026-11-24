//! `gmns_driving.chains`: the chains of the source db's `driving.chains`, made for the GMNS links
//! (docs/design/sim_from_gmns.md, "Chains"). `duckosm sumo --merge` and `duckosm matsim --merge` read it from a GMNS db.
//!
//! A chain is cut where the GMNS lanes change (a merged SUMO edge has the lanes of its last link), and each GMNS driving link
//! is in one chain (a bus-only link is a chain of one link). Then each chain is cut at each node inside it where another chain
//! ends, so the two directions of a two-way road end at the same nodes (as `duckosm_core::chains::cut_at_ends`).

use crate::Result;
use duckdb::Connection;
use std::path::Path;

/// The cut at the ends of the other chains, from `{T}(old, seg, link_id, i)`: the same rows with `seg` cut again at each node
/// inside a chain where a chain starts or ends. `gmns.py` `_CUT_AT_ENDS` is the same text.
const CUT_AT_ENDS: &str = "WITH n AS (SELECT c.*, l.from_node_id AS fn, l.to_node_id AS tn FROM {T} c JOIN gmns_driving.link l USING (link_id)),
       stops AS (SELECT arg_min(fn, i) AS node FROM n GROUP BY old, seg UNION SELECT arg_max(tn, i) FROM n GROUP BY old, seg),
       f AS (SELECT n.*, CASE WHEN lag(n.i) OVER w IS NULL OR lag(n.seg) OVER w <> n.seg OR n.fn IN (SELECT node FROM stops)
                              THEN 1 ELSE 0 END AS brk FROM n WINDOW w AS (PARTITION BY n.old ORDER BY n.i))
       SELECT old, sum(brk) OVER (PARTITION BY old ORDER BY i) AS seg, link_id, i FROM f";

/// Write `gmns_driving.chains` into the GMNS db `gmns` from the duckOSM db `source`. Gives the number of chains; None when
/// there is no `gmns_driving.link`. A source with no `driving.chains` (an old build, or `--no-chains`): each link is a chain
/// of one link, so `--merge` still works and changes nothing.
pub fn add_chains(source: &Path, gmns: &Path) -> Result<Option<usize>> {
    let con = Connection::open(gmns).map_err(|e| format!("{}: {e}", gmns.display()))?;
    let x = |sql: &str| con.execute_batch(sql).map_err(|e| format!("{e}\n{sql}"));
    x("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")?;
    x(&format!("ATTACH '{}' AS s (READ_ONLY)", source.display().to_string().replace('\'', "''")))?;
    let has = |db: &str, sch: &str, t: &str| -> Result<bool> {
        con.query_row(
            "SELECT count(*) FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?",
            [db, sch, t],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
    };
    let me: String = con.query_row("SELECT current_database()", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !has(&me, "gmns_driving", "link")? {
        return Ok(None);
    }
    let source_chains = if has("s", "driving", "chains")? {
        "SELECT chain_id, edge_ids FROM s.driving.chains"
    } else {
        eprintln!("GMNS: the source db has no driving.chains (build it again with this duckOSM version): each link is a chain of one link");
        "SELECT NULL::BIGINT AS chain_id, NULL::BIGINT[] AS edge_ids WHERE false"
    };
    // the lanes of each link, left to right: a chain is cut where they change
    x("CREATE OR REPLACE TEMP TABLE _lane_sig AS SELECT link_id, string_agg(lane_num::VARCHAR || ':' || coalesce(allowed_uses, '') || ':' \
       || coalesce(round(width, 2)::VARCHAR, ''), '|' ORDER BY lane_num) AS sig FROM gmns_driving.lane GROUP BY link_id")?;
    // the links of each chain, in order; `seg`: the part of the chain after each cut. A link that GMNS does not have cuts
    // the chain too. The links in no chain (bus-only) are chains of one link.
    x(&format!("CREATE OR REPLACE TABLE main._chain_links AS
       WITH u AS (SELECT c.chain_id AS old, m.e AS link_id, m.i FROM ({source_chains}) c, unnest(c.edge_ids) WITH ORDINALITY AS m(e, i)
                  WHERE m.e IN (SELECT link_id FROM gmns_driving.link)),
       f AS (SELECT u.*, CASE WHEN lag(u.i) OVER w IS NULL OR lag(u.i) OVER w <> u.i - 1 OR g.sig IS DISTINCT FROM lag(g.sig) OVER w
                              THEN 1 ELSE 0 END AS brk
             FROM u LEFT JOIN _lane_sig g USING (link_id) WINDOW w AS (PARTITION BY u.old ORDER BY u.i))
       SELECT old, sum(brk) OVER (PARTITION BY old ORDER BY i) AS seg, link_id, i FROM f
       UNION ALL
       SELECT link_id, 1, link_id, 1 FROM gmns_driving.link WHERE link_id NOT IN (SELECT link_id FROM u)"))?;
    // the two directions of a two-way road end at the same nodes: a cut at each node inside a chain where another chain ends
    x(&format!("CREATE OR REPLACE TABLE main._chain_cut AS {};
       DROP TABLE main._chain_links; ALTER TABLE main._chain_cut RENAME TO _chain_links", CUT_AT_ENDS.replace("{T}", "main._chain_links")))?;
    // the id of each chain: duckosm_core's chain_id of its links (the Level 2 chain_id when the chain is not cut)
    x("CREATE OR REPLACE TABLE main._chain_ids(old BIGINT, seg BIGINT, chain_id BIGINT)")?;
    {
        let mut st = con
            .prepare("SELECT old, seg, list(link_id ORDER BY i) FROM main._chain_links GROUP BY old, seg")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                let ids: Vec<i64> = match r.get::<_, duckdb::types::Value>(2)? {
                    duckdb::types::Value::List(l) => l.into_iter().filter_map(|v| if let duckdb::types::Value::BigInt(i) = v { Some(i) } else { None }).collect(),
                    _ => Vec::new(),
                };
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, ids))
            })
            .map_err(|e| e.to_string())?;
        let mut a = con.appender("_chain_ids").map_err(|e| e.to_string())?;
        for row in rows {
            let (old, seg, ids) = row.map_err(|e| e.to_string())?;
            a.append_row(duckdb::params![old, seg, duckosm_core::chains::chain_id(&ids)]).map_err(|e| e.to_string())?;
        }
        a.flush().map_err(|e| e.to_string())?;
    }
    x("CREATE OR REPLACE TABLE gmns_driving.chains AS
       SELECT d.chain_id, arg_min(l.from_node_id, m.i) AS from_node_id, arg_max(l.to_node_id, m.i) AS to_node_id,
              list(m.link_id ORDER BY m.i) AS link_ids, sum(l.length)::DOUBLE AS length,
              ST_RemoveRepeatedPoints(ST_MakeLine(flatten(list([p.geom FOR p IN ST_Dump(ST_Points(l.geom))] ORDER BY m.i)))) AS geom
       FROM main._chain_links m JOIN main._chain_ids d USING (old, seg) JOIN gmns_driving.link l USING (link_id)
       GROUP BY d.chain_id ORDER BY d.chain_id;
       DROP TABLE main._chain_links; DROP TABLE main._chain_ids; DETACH s;")?;
    let n: i64 = con.query_row("SELECT count(*) FROM gmns_driving.chains", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    Ok(Some(n as usize))
}
