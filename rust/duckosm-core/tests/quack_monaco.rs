//! `duckosm_core::quack` on Monaco gives the features table of QuackOSM 0.20.0 (docs/rust/quack.md). The numbers are the
//! ones of QuackOSM's `convert_pbf_to_parquet` (default options) on `tests/data/monaco.osm.pbf`: the rows of each kind,
//! the rows of each geometry type, and the area of the polygons. A row-by-row check of the two files (the same ids, tags
//! and ST_Equals geometry for all 9 586 rows) is `cargo run --example quack_diff` of duckosm-duckdb.
#![cfg(feature = "pbf")]
use std::collections::BTreeMap;

/// The WKB type (1 point, 2 line, 3 polygon, 6 multipolygon) and the area of a polygon or a multipolygon.
fn kind_and_area(wkb: &[u8]) -> (u32, f64) {
    let u32_at = |i: usize| u32::from_le_bytes(wkb[i..i + 4].try_into().unwrap());
    let f64_at = |i: usize| f64::from_le_bytes(wkb[i..i + 8].try_into().unwrap());
    // the area of the polygon at `at` (shell minus holes), and the byte after it
    let polygon = |mut at: usize| -> (f64, usize) {
        let rings = u32_at(at + 5) as usize;
        at += 9;
        let mut area = 0.0;
        for r in 0..rings {
            let n = u32_at(at) as usize;
            at += 4;
            let mut a = 0.0;
            for i in 0..n - 1 {
                let (x0, y0) = (f64_at(at + 16 * i), f64_at(at + 16 * i + 8));
                let (x1, y1) = (f64_at(at + 16 * (i + 1)), f64_at(at + 16 * (i + 1) + 8));
                a += x0 * y1 - x1 * y0;
            }
            area += if r == 0 { a.abs() / 2.0 } else { -a.abs() / 2.0 };
            at += 16 * n;
        }
        (area, at)
    };
    match u32_at(1) {
        3 => (3, polygon(0).0),
        6 => {
            let (mut at, mut area) = (9, 0.0);
            for _ in 0..u32_at(5) {
                let (a, next) = polygon(at);
                area += a;
                at = next;
            }
            (6, area)
        }
        k => (k, 0.0),
    }
}

#[test]
fn monaco_equals_quackosm() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let mut kinds: BTreeMap<u32, (usize, f64)> = BTreeMap::new();
    let n = duckosm_core::quack::features(open, 1 << 20, |rows| {
        for r in rows {
            let (k, a) = kind_and_area(&r.geometry);
            let e = kinds.entry(k).or_default();
            e.0 += 1;
            e.1 += a;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!((n.nodes, n.ways, n.relations), (3957, 5573, 56));
    assert_eq!(kinds[&1].0, 3957, "points");
    assert_eq!(kinds[&2].0, 3722, "lines");
    assert_eq!(kinds[&3].0, 1905, "polygons");
    assert_eq!(kinds[&6].0, 2, "multipolygons");
    // the sum of the areas, to 1e-8 of it (GEOS ST_Area adds the terms in another order)
    let near = |a: f64, b: f64| (a - b).abs() < 1e-8 * b;
    assert!(near(kinds[&3].1, 0.0005955805303099766), "area of the polygons {}", kinds[&3].1);
    assert!(near(kinds[&6].1, 0.00029927139816998953), "area of the multipolygons {}", kinds[&6].1);
}
