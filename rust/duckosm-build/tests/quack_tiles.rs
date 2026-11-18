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
    // three tests at once: macOS gives 256 open files
    duckosm_core::util::raise_open_files();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let mut whole: Vec<Row> = Vec::new();
    duckosm_core::quack::features(open, 1 << 20, None, |rows| {
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
    duckosm_core::util::raise_open_files();
    use duckosm_core::quack::{tiles_of_file, TileSize, WORLD};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let open = || {
        Ok(std::io::BufReader::new(
            std::fs::File::open(&path).map_err(|e| e.to_string())?,
        ))
    };
    let mut whole: Vec<Row> = Vec::new();
    duckosm_core::quack::features(open, 1 << 20, None, |rows| {
        // the center from the WKB (the key of the merge) is the center of the row (the key of the sort in the tile)
        for f in &rows {
            let c = duckosm_core::quack::wkb_center(&f.geometry).unwrap();
            assert_eq!((c.0.to_bits(), c.1.to_bits()), (f.center.0.to_bits(), f.center.1.to_bits()), "{}", f.feature_id);
        }
        whole.extend(rows.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    whole.sort();
    let tiles = tiles_of_file(&open, TileSize::Nodes(2_000)).unwrap();
    assert!(tiles.len() > 4, "small tiles");
    let world = duckosm_core::boundary::Boundary::parse(WORLD).unwrap();
    let mut files: Vec<Vec<u8>> = vec![Vec::new(); tiles.len()];
    let counts = duckosm_core::tilecut::cut_to_sink(
        open,
        &world,
        &tiles,
        &["multipolygon", "boundary"],
        false,
        &mut |t, b| {
            files[t as usize].extend_from_slice(b);
            Ok(())
        },
    )
    .unwrap();
    let (mut parts, mut sorted) = (Vec::new(), Vec::new());
    for (i, f) in files.iter().enumerate() {
        if counts[i].nodes == 0 {
            continue;
        }
        for (sort, list) in [(false, &mut parts), (true, &mut sorted)] {
            let mut part = Vec::new();
            duckosm_build::quack_tile_to(
                || Ok(std::io::Cursor::new(&f[..])),
                tiles[i],
                &mut part,
                sort,
                None,
            )
            .unwrap();
            list.push(std::io::Cursor::new(part));
        }
    }
    let read = |file: Vec<u8>| {
        let mut rows = Vec::new();
        duckosm_parquet::quack::read_quack_bytes(file, |b| {
            rows.extend(duckosm_parquet::quack::features_of(&b)?);
            Ok(())
        })
        .unwrap();
        rows
    };
    let mut joined = Vec::new();
    let n = duckosm_build::quack_join_to(parts, &mut joined, false, &mut |_, _| {}).unwrap();
    assert_eq!(n, whole.len());
    // the rows in the order of the parts: the row numbers of the sort
    let unsorted = read(joined);
    let mut got: Vec<Row> = unsorted.iter().cloned().map(row).collect();
    got.sort();
    assert!(got == whole, "the join of the parts equals the whole file");
    // with the sort: the sorted parts merged give the order of str_order on the rows in the order of the parts
    // each part is dropped when the merge has read all its rows, before the merge ends (the browser then deletes its file)
    struct Part(std::io::Cursor<Vec<u8>>, std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<std::cell::Cell<usize>>);
    impl std::io::Read for Part {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(b)
        }
    }
    impl std::io::Seek for Part {
        fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(to)
        }
    }
    impl Drop for Part {
        fn drop(&mut self) {
            if !self.1.get() {
                self.2.set(self.2.get() + 1);
            }
        }
    }
    let (ended, early) = (std::rc::Rc::new(std::cell::Cell::new(false)), std::rc::Rc::new(std::cell::Cell::new(0)));
    let count = sorted.len();
    let sorted: Vec<Part> = sorted.into_iter().map(|c| Part(c, ended.clone(), early.clone())).collect();
    let mut joined = Vec::new();
    let mut last = (0, 0);
    let n = duckosm_build::quack_join_to(sorted, &mut joined, true, &mut |m, t| {
        last = (m, t);
        ended.set(m == t);
    })
    .unwrap();
    assert_eq!((n, last), (whole.len(), (n, n)));
    assert_eq!(early.get(), count, "each part is dropped before the merge ends");
    let centers: Vec<_> = unsorted.iter().map(|f| f.center).collect();
    let want: Vec<String> = duckosm_core::quack::str_order(&centers)
        .into_iter()
        .map(|i| unsorted[i].feature_id.clone())
        .collect();
    let got: Vec<String> = read(joined).into_iter().map(|f| f.feature_id).collect();
    assert!(got == want, "the sorted join has the order of str_order");
}

/// quack with the OSM fixes (`tests/data/osm_fixes_quack.yaml`): the whole file, by tiles, and the tile workers of the
/// browser (the fixes as JSON, placed on the tiles) give the same rows, with the fixes in them.
#[test]
fn monaco_fixes() {
    duckosm_core::util::raise_open_files();
    use duckosm_core::quack::{features, features_by_tiles, tiles_of_file, QuackFixes, TileSize, WORLD};
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let path = data.join("monaco.osm.pbf");
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let osm = duckosm_build::config::read_osm_fixes(&data.join("osm_fixes_quack.yaml")).unwrap();
    let fixes = || QuackFixes::new(&osm, open).unwrap();
    let mut whole: Vec<Row> = Vec::new();
    features(open, 1 << 20, Some(&fixes()), |rows| {
        whole.extend(rows.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    whole.sort();
    let get = |id: &str| whole.iter().find(|r| r.0 == id);
    let tag = |id: &str, k: &str| get(id).and_then(|r| r.1.iter().find(|t| t.0 == k)).map(|t| t.1.as_str());
    assert!(get("way/23715051").is_none(), "removed");
    assert_eq!(tag("way/4227277", "surface"), Some("cobblestone"));
    assert_eq!(tag("way/503462470", "oneway"), Some("yes"));
    assert_eq!(tag("way/503462470", "lanes"), Some("3"));
    assert_eq!(tag("way/4227277", "sidewalk"), Some("both"));
    assert_eq!(tag("way/4227277", "lit"), None);
    assert_eq!(tag("way/9000000000001", "name"), Some("Test"));
    assert_eq!(tag("way/9000000000001", "highway"), Some("footway"));
    assert_eq!(tag("way/9000000000002", "highway"), Some("service"));
    assert_eq!(tag("node/21911886", "crossing"), Some("marked"));
    assert_eq!(tag("node/21911886", "crossing_ref"), None);
    assert_eq!(tag("relation/393226", "name"), Some("Test relation"));
    // with no fixes, the rows differ
    let mut plain: Vec<Row> = Vec::new();
    features(open, 1 << 20, None, |rows| {
        plain.extend(rows.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    assert!(plain.iter().any(|r| r.0 == "way/23715051"));
    for size in [TileSize::Nodes(2_000), TileSize::Zoom(14)] {
        let mut tiled: Vec<Row> = Vec::new();
        features_by_tiles(open, 1 << 20, size, Some(fixes()), |rows| {
            tiled.extend(rows.into_iter().map(row));
            Ok(())
        })
        .unwrap();
        tiled.sort();
        assert!(tiled == whole, "by tiles {size:?}");
    }
    // the browser: the fixes go to each worker as JSON, placed on the tiles of the plan
    let tiles = tiles_of_file(&open, TileSize::Zoom(14)).unwrap();
    let mut placed = fixes();
    placed.place(&tiles);
    let json = placed.to_json();
    let world = duckosm_core::boundary::Boundary::parse(WORLD).unwrap();
    let files = duckosm_core::tilecut::cut_to_store(&open, &world, &tiles, &["multipolygon", "boundary"]).unwrap();
    let mut parts = Vec::new();
    for (i, (tile, c)) in files.tiles.iter().enumerate() {
        if c.nodes == 0 {
            continue;
        }
        let worker = QuackFixes::from_json(&json).unwrap();
        let mut part = Vec::new();
        let open_tile = || Ok(std::io::BufReader::new(files.reader(i)));
        duckosm_build::quack_tile_to(open_tile, *tile, &mut part, false, Some(&worker)).unwrap();
        parts.push(std::io::Cursor::new(part));
    }
    let mut joined = Vec::new();
    duckosm_build::quack_join_to(parts, &mut joined, false, &mut |_, _| {}).unwrap();
    let mut got: Vec<Row> = Vec::new();
    duckosm_parquet::quack::read_quack_bytes(joined, |b| {
        got.extend(duckosm_parquet::quack::features_of(&b)?.into_iter().map(row));
        Ok(())
    })
    .unwrap();
    got.sort();
    assert!(got == whole, "the tile workers");
}
