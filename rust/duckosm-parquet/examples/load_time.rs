//! Where the time of the load step goes: `load_time <file.osm.pbf> <read|decode|parts|load> [out_dir]`.
//! read: only read the blocks; decode: decode the blocks (no rows); parts: build the Arrow parts; load: load_raw.
use duckosm_parquet::{load_raw, RawColumns};
use std::time::Instant;

struct Nothing;
impl duckosm_core::raw::RawSink for Nothing {
    fn node<'a>(&mut self, _: i64, _: f64, _: f64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>) {}
    fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {}
    fn relation<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = (i64, duckosm_core::graph::MemberType, &'a str)>) {}
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = || std::io::BufReader::new(std::fs::File::open(&a[1]).unwrap());
    let t = Instant::now();
    let k = 2 * duckosm_core::pbf::threads();
    let fits = |_: usize, r: &[usize]| r.len() < k;
    match a[2].as_str() {
        "read" => { duckosm_core::pbf::stream_raw(f(), fits, || Nothing, |_| Ok(()), |_, _| Ok(())).unwrap(); }
        "decode" => {
            // the elements are visited, but nothing is kept
            struct Visit;
            impl duckosm_core::raw::RawSink for Visit {
                fn node<'a>(&mut self, _: i64, _: f64, _: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) { t.for_each(drop) }
                fn way<'a>(&mut self, _: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, r: &mut dyn Iterator<Item = i64>) { t.for_each(drop); r.for_each(drop) }
                fn relation<'a>(&mut self, _: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, m: &mut dyn Iterator<Item = (i64, duckosm_core::graph::MemberType, &'a str)>) { t.for_each(drop); m.for_each(drop) }
            }
            duckosm_core::pbf::stream_raw(f(), fits, || Visit, |_| Ok(()), |_, _| Ok(())).unwrap();
        }
        "parts" => { duckosm_core::pbf::stream_raw(f(), fits, RawColumns::default, RawColumns::finish, |_, _| Ok(())).unwrap(); }
        _ => {
            let out = std::path::PathBuf::from(&a[3]);
            load_raw(f(), 1_000_000_000, duckosm_core::pbf::threads(), |n| Ok(std::io::BufWriter::new(std::fs::File::create(out.join(n)).unwrap()))).unwrap();
        }
    }
    println!("{}: {:.1} s", a[2], t.elapsed().as_secs_f64());
}
