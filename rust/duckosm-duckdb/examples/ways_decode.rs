//! The decode of the way blocks alone (no DuckDB): `ways_decode <file.osm.pbf> <read|visit|arrow>`.
//! read: read the blocks; visit: decode and visit each way (nothing kept); arrow: build the Arrow parts.
use duckosm_core::pbf::{blob_from_bytes, block_index, kind_range};
use rayon::prelude::*;
use std::os::unix::fs::FileExt;

struct Visit(usize);
impl duckosm_core::raw::RawSink for Visit {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        self.0 += 1 + t.count();
    }
    fn way<'a>(&mut self, _: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, r: &mut dyn Iterator<Item = i64>) {
        self.0 += t.count() + r.count();
    }
    fn relation<'a>(&mut self, _: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, m: &mut dyn Iterator<Item = (i64, duckosm_core::graph::MemberType, &'a str)>) {
        self.0 += t.count() + m.count();
    }
}

/// The zlib bytes of a block (the Blob message after the 4-byte length and the BlobHeader).
fn zlib_data(b: &[u8]) -> &[u8] {
    let h = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut blob = &b[4 + h..];
    let varint = |x: &mut &[u8]| {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let c = x[0];
            *x = &x[1..];
            v |= ((c & 0x7f) as u64) << s;
            s += 7;
            if c < 0x80 {
                return v;
            }
        }
    };
    while !blob.is_empty() {
        let key = varint(&mut blob);
        match key & 7 {
            0 => {
                varint(&mut blob);
            }
            2 => {
                let len = varint(&mut blob) as usize;
                if key >> 3 == 3 {
                    return &blob[..len];
                }
                blob = &blob[len..];
            }
            _ => panic!("wire type"),
        }
    }
    panic!("no zlib data")
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = std::fs::File::open(&a[1]).unwrap();
    let index = block_index(std::io::BufReader::new(std::fs::File::open(&a[1]).unwrap())).unwrap();
    let read = |(at, len): (u64, usize)| {
        let mut b = vec![0u8; len];
        f.read_exact_at(&mut b, at).unwrap();
        blob_from_bytes(&b).unwrap()
    };
    // "all...": all blocks of the file (osmium fileinfo -e reads all of them too)
    let range = if a[2].ends_with("all") {
        0..index.blocks.len()
    } else {
        kind_range(&index, 1, |i| Ok(read(index.blocks[i]))).unwrap()
    };
    let blocks = &index.blocks[range];
    let t = std::time::Instant::now();
    let n: usize = blocks
        .par_iter()
        .map(|&b| {
            if a[2] == "inflate" {
                let mut bytes = vec![0u8; b.1];
                f.read_exact_at(&mut bytes, b.0).unwrap();
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(zlib_data(&bytes)), &mut out).unwrap();
                return out.len();
            }
            if a[2].starts_with("fast") {
                let mut bytes = vec![0u8; b.1];
                f.read_exact_at(&mut bytes, b.0).unwrap();
                let mut buf = duckosm_core::pbfraw::Buffers::default();
                if a[2] == "fast" || a[2] == "fastall" {
                    let mut v = Visit(0);
                    let keep = if a[2] == "fastall" { [true; 3] } else { [false, true, false] };
                    duckosm_core::pbfraw::decode_block(&bytes, keep, &mut buf, &mut v).unwrap();
                    return v.0;
                }
                let mut c = duckosm_parquet::RawColumns::only(1);
                duckosm_core::pbfraw::decode_block(&bytes, [false, true, false], &mut buf, &mut c).unwrap();
                return c.finish().unwrap().batches[1].num_rows();
            }
            let blob = read(b);
            match a[2].as_str() {
                "read" => 1,
                "parse" => blob.to_primitiveblock().unwrap().groups().count(),
                "refs" => {
                    let b = blob.to_primitiveblock().unwrap();
                    b.groups().map(|g| g.ways().map(|w| w.refs().count()).sum::<usize>()).sum()
                }
                "visit" => {
                    let mut v = Visit(0);
                    duckosm_core::pbf::raw_block(&blob, &mut v).unwrap();
                    v.0
                }
                _ => {
                    let mut c = duckosm_parquet::RawColumns::only(1);
                    duckosm_core::pbf::raw_block(&blob, &mut c).unwrap();
                    c.finish().unwrap().batches[1].num_rows()
                }
            }
        })
        .sum();
    println!("{} ({} blocks, {n}): {:.1} s", a[2], blocks.len(), t.elapsed().as_secs_f64());
}
