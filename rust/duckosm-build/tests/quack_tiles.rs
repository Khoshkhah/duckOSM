//! Tile tables of quack on Monaco (docs/rust/quack.md, "Tile tables"): each tile is made alone from its own file; the
//! union (the rows with `owner`) equals the table of the whole file, and a node is only in the table of its tile.
use duckosm_core::quack::Feature;
use duckosm_core::tiles::Tile;
use std::collections::BTreeSet;

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
    let (mut owned, mut seen): (Vec<Row>, BTreeSet<String>) = (Vec::new(), BTreeSet::new());
    for f in &files {
        let mut parts: Vec<_> = f.strip_prefix(&dir).unwrap().iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let y: u32 = parts.pop().unwrap().trim_end_matches(".parquet").parse().unwrap();
        let x: u32 = parts.pop().unwrap().parse().unwrap();
        let tile = Tile { z, x, y };
        duckosm_parquet::quack::read_tile_table(f, |b| {
            use arrow_array::cast::AsArray;
            let owner = b.column_by_name("owner").unwrap().as_boolean().clone();
            let plain = arrow_array::RecordBatch::try_new(duckosm_parquet::quack::schema(), b.columns()[..3].to_vec()).unwrap();
            for (i, f) in duckosm_parquet::quack::features_of(&plain).unwrap().into_iter().enumerate() {
                if f.feature_id.starts_with("node/") {
                    assert_eq!(Tile::of(z, f.center.0, f.center.1), tile, "{} in the table of another tile", f.feature_id);
                    assert!(owner.value(i), "{}: a node is owned by its tile", f.feature_id);
                }
                seen.insert(f.feature_id.clone());
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
    assert_eq!(seen.len(), whole.len(), "each row is in a table");
    assert!(n.stored > n.rows, "the ways at the edges are in more than one table");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The flow of the browser's tile workers on Monaco: the cut gives each tile file to a sink, each tile alone makes its
/// part (`quack_tile_to`), and the join of the parts equals the table of the whole file.
#[test]
fn monaco_tile_workers() {
    use duckosm_core::quack::{tiles_of_file, TileSize, WORLD};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let mut whole: Vec<Row> = Vec::new();
    duckosm_core::quack::features(open, 1 << 20, |rows| {
        whole.extend(rows.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    whole.sort();
    let tiles = tiles_of_file(&open, TileSize::Nodes(2_000)).unwrap();
    assert!(tiles.len() > 4, "small tiles");
    let world = duckosm_core::boundary::Boundary::parse(WORLD).unwrap();
    let mut files: Vec<Vec<u8>> = vec![Vec::new(); tiles.len()];
    let counts = duckosm_core::tilecut::cut_to_sink(open, &world, &tiles, &["multipolygon", "boundary"], &mut |t, b| {
        files[t as usize].extend_from_slice(b);
        Ok(())
    })
    .unwrap();
    let mut parts = Vec::new();
    for (i, f) in files.iter().enumerate() {
        if counts[i].nodes == 0 {
            continue;
        }
        let mut part = Vec::new();
        duckosm_build::quack_tile_to(|| Ok(std::io::Cursor::new(&f[..])), tiles[i], &mut part).unwrap();
        parts.push(part);
    }
    let mut joined = Vec::new();
    let n = duckosm_build::quack_join_to(parts.into_iter().map(Ok), &mut joined, false).unwrap();
    assert_eq!(n, whole.len());
    let mut got: Vec<Row> = Vec::new();
    duckosm_parquet::quack::read_quack_bytes(joined, |b| {
        got.extend(duckosm_parquet::quack::features_of(&b)?.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    got.sort();
    assert!(got == whole, "the join of the parts equals the whole file");
}
