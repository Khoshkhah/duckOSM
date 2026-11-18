//! `duckosm quack --pmtiles` on `tests/data/monaco_sample.osm.pbf` equals the golden file
//! `tests/data/monaco_sample_shortbread.txt`: one line for each tile (z/x/y, the bytes of the MVT tile with no gzip, and
//! their SHA-256), in the order of the tile ids. The file by tiles of zoom 12 gives the same tiles.
//!
//! The golden file is made one time with `DUCKOSM_WRITE_GOLDEN=1`. A failure after a change means that the change altered
//! the tiles: never write the golden file again to make the test pass.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

fn varint(b: &[u8], i: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut s = 0;
    loop {
        let c = b[*i];
        *i += 1;
        v |= ((c & 0x7f) as u64) << s;
        if c < 0x80 {
            return v;
        }
        s += 7;
    }
}

fn gunzip(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut out).expect("gzip");
    out
}

/// The tiles of a PMTiles file: (tile id, MVT bytes with no gzip), from the root and the leaf directories.
fn tiles(b: &[u8]) -> Vec<(u64, Vec<u8>)> {
    assert_eq!(&b[..8], b"PMTiles\x03");
    let h = |i: usize| u64::from_le_bytes(b[8 + 8 * i..16 + 8 * i].try_into().unwrap()) as usize;
    let (root, root_len, leaves, data) = (h(0), h(1), h(4), h(6));
    let mut out = Vec::new();
    let mut dirs = vec![(root, root_len, 0usize)];
    while let Some((at, len, base)) = dirs.pop() {
        let d = gunzip(&b[base + at..base + at + len]);
        let mut i = 0;
        let n = varint(&d, &mut i) as usize;
        let mut ids = Vec::new();
        let mut last = 0;
        for _ in 0..n {
            last += varint(&d, &mut i);
            ids.push(last);
        }
        let runs: Vec<u64> = (0..n).map(|_| varint(&d, &mut i)).collect();
        let lens: Vec<u64> = (0..n).map(|_| varint(&d, &mut i)).collect();
        let mut offs: Vec<u64> = Vec::new();
        for k in 0..n {
            let v = varint(&d, &mut i);
            offs.push(if v == 0 && k > 0 { offs[k - 1] + lens[k - 1] } else { v - 1 });
        }
        for k in 0..n {
            if runs[k] == 0 {
                dirs.push((offs[k] as usize, lens[k] as usize, leaves));
            } else {
                let t = &b[data + offs[k] as usize..data + (offs[k] + lens[k]) as usize];
                for r in 0..runs[k] {
                    out.push((ids[k] + r, gunzip(t)));
                }
            }
        }
    }
    out.sort_by_key(|t| t.0);
    out
}

fn lines(pbf: &Path, tile_zoom: Option<u8>) -> String {
    let dir = std::env::temp_dir().join(format!("duckosm-pmtiles-{}-{:?}", std::process::id(), tile_zoom));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("monaco.pmtiles");
    let n = duckosm_build::quack_pmtiles::quack_pmtiles(pbf, &out, tile_zoom.map(duckosm_build::TileSize::Zoom), None).expect("quack --pmtiles");
    assert!(n.features > 0 && n.tiles > 0);
    let b = std::fs::read(&out).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let mut s = String::new();
    for (id, t) in tiles(&b) {
        let (z, x, y) = duckosm_core::pmtiles::tile_zxy(id);
        let hex: String = Sha256::digest(&t).iter().map(|b| format!("{b:02x}")).collect();
        s += &format!("{z}/{x}/{y} {} {hex}\n", t.len());
    }
    s
}

#[test]
fn monaco_sample_equals_the_golden_file() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let got = lines(&data.join("monaco_sample.osm.pbf"), None);
    let golden = data.join("monaco_sample_shortbread.txt");
    if std::env::var_os("DUCKOSM_WRITE_GOLDEN").is_some() {
        std::fs::write(&golden, &got).unwrap();
    }
    let want = std::fs::read_to_string(&golden).expect("the golden file");
    assert_eq!(got, want, "the tiles differ from the golden file");
    assert_eq!(lines(&data.join("monaco_sample.osm.pbf"), Some(12)), want, "by tiles of zoom 12");
}
