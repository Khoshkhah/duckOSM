//! The database tools of duckOSM in Rust (docs/design/all_in_rust.md, group B). `duckosm <tool> ...` goes to
//! [`cli::run`]; [`cli::COMMANDS`] has the names.

pub mod admin;
pub mod cli;
pub mod dem_sources;
pub mod elevation;
pub mod extract;
pub mod info;
pub mod json;
pub mod way;

pub type Result<T> = std::result::Result<T, String>;

/// The commented config template of duckOSM (`duckosm init-config`), the same file as the Python package ships.
pub const CONFIG_TEMPLATE: &str = include_str!("../../../src/duckosm/templates/config.yaml");

/// Open a database read-only.
pub fn open_read_only(db: &std::path::Path) -> Result<duckdb::Connection> {
    let cfg = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly).map_err(|e| e.to_string())?;
    duckdb::Connection::open_with_flags(db, cfg).map_err(|e| format!("{}: {e}", db.display()))
}

/// The mode schemas of the database that have an `edges` table, in the order driving, walking, cycling (duckOSM
/// `query._mode_schemas`), optionally only those of `only`.
pub fn mode_schemas(con: &duckdb::Connection, only: &[String]) -> Result<Vec<String>> {
    let mut st = con
        .prepare("SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_catalog = current_database() AND table_name = 'edges'")
        .map_err(|e| e.to_string())?;
    let have: Vec<String> = st.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())?;
    let want: Vec<String> = if only.is_empty() { ["driving", "walking", "cycling"].map(String::from).to_vec() } else { only.to_vec() };
    Ok(want.into_iter().filter(|m| have.contains(m)).collect())
}
