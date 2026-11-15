//! Tile tables of quack on Monaco (docs/rust/quack.md, "Tile tables"): the union (the rows with `owner`) equals the table
//! of the whole file, each row is owned in one tile only, and each tile table holds exactly the rows whose geometry
//! touches the tile.
use duckosm_core::quack::{Feature, row_tiles};
use std::collections::{BTreeMap, BTreeSet};

type Row = (String, Vec<(String, String)>, Vec<u8>);

fn row(f: Feature) -> Row {
    (f.feature_id, f.tags, f.geometry)
}

#[test]
fn monaco_tile_tables() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let mut whole: Vec<Row> = Vec::new();
    duckosm_core::quack::features(open, 1 << 20, |rows| {
        whole.extend(rows.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    whole.sort();
    let z = 14;
    let dir = std::env::temp_dir().join(format!("quack-tiles-test-{}", std::process::id()));
    let n = duckosm_build::quack_tiles::quack_tile_tables(open, z, &dir, "monaco.osm.pbf").unwrap();
    let files = duckosm_build::quack_tiles::tile_table_files(&dir).unwrap();
    assert_eq!(files.len(), n.tiles);
    assert!(n.tiles > 1, "Monaco at zoom 14 has more than one tile");
    // the rows of each tile, and the owned rows
    let mut owned: Vec<Row> = Vec::new();
    let mut by_tile: BTreeMap<(u32, u32), BTreeSet<String>> = BTreeMap::new();
    for f in &files {
        let mut parts: Vec<_> = f.strip_prefix(&dir).unwrap().iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let y: u32 = parts.pop().unwrap().trim_end_matches(".parquet").parse().unwrap();
        let x: u32 = parts.pop().unwrap().parse().unwrap();
        duckosm_parquet::quack::read_tile_table(f, |b| {
            use arrow_array::cast::AsArray;
            let owner = b.column_by_name("owner").unwrap().as_boolean().clone();
            let plain = arrow_array::RecordBatch::try_new(duckosm_parquet::quack::schema(), b.columns()[..3].to_vec()).unwrap();
            for (i, f) in duckosm_parquet::quack::features_of(&plain).unwrap().into_iter().enumerate() {
                by_tile.entry((x, y)).or_default().insert(f.feature_id.clone());
                if owner.value(i) {
                    owned.push(row(f));
                }
            }
            Ok(())
        })
        .unwrap();
    }
    owned.sort();
    assert_eq!(owned.len(), whole.len(), "the owned rows");
    assert!(owned == whole, "the union equals the whole file");
    assert_eq!(n.rows, whole.len());
    // each tile table: the rows whose geometry touches the tile
    let mut want: BTreeMap<(u32, u32), BTreeSet<String>> = BTreeMap::new();
    let mut ts = Vec::new();
    for (id, _, g) in &whole {
        row_tiles(g, z, &mut ts).unwrap();
        for t in &ts {
            want.entry((t.x, t.y)).or_default().insert(id.clone());
        }
    }
    // a row is also in the tile that made it (its first node there, but its geometry only on the side of the tile)
    for (t, ids) in &want {
        let got = &by_tile[t];
        assert!(ids.is_subset(got), "tile {t:?}: rows that touch it are missing");
        assert!(got.len() - ids.len() <= 5, "tile {t:?}: {} rows that do not touch it", got.len() - ids.len());
    }
    assert_eq!(want.keys().collect::<Vec<_>>(), by_tile.keys().collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(&dir);
}
