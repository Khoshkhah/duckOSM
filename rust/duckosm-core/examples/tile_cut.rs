//! The tile cut of a region file, and its test against the cut of each tile alone.
//!
//!   tile_cut REGION BOUNDARY ZOOM OUT            one read: a file for each tile (OUT/z_x_y.osm.pbf)
//!   tile_cut REGION BOUNDARY ZOOM OUT reference [z/x/y,...]  the cut of each tile alone (cut_where, the boundary and the tile)
//!   tile_cut compare DIR_A DIR_B                 the same files with the same elements?

use duckosm_core::boundary::Boundary;
use duckosm_core::cutstream::cut_where;
use duckosm_core::graph::MemberType;
use duckosm_core::pbfraw::{decode_block, read_block, Buffers};
use duckosm_core::raw::RawSink;
use duckosm_core::tilecut::cut_to_tiles;
use duckosm_core::tiles::{tiles, Tile};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// All elements of a file, in its order.
#[derive(Default, PartialEq)]
struct All {
    nodes: Vec<(i64, u64, u64, Vec<(String, String)>)>,
    ways: Vec<(i64, Vec<(String, String)>, Vec<i64>)>,
    rels: Vec<(i64, Vec<(String, String)>, Vec<(i64, u8, String)>)>,
}

impl RawSink for All {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        self.nodes.push((id, lat.to_bits(), lon.to_bits(), t.map(|(k, v)| (k.into(), v.into())).collect()));
    }
    fn way<'a>(&mut self, id: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, r: &mut dyn Iterator<Item = i64>) {
        self.ways.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), r.collect()));
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        m: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        let m = m.map(|(i, ty, r)| (i, ty as u8, r.to_string())).collect();
        self.rels.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), m));
    }
}

fn elements(path: &Path) -> All {
    let (mut all, mut buf) = (All::default(), Buffers::default());
    let mut r = std::io::BufReader::new(File::open(path).unwrap());
    while let Some(b) = read_block(&mut r).unwrap() {
        decode_block(&b, [true; 3], &mut buf, &mut all).unwrap();
    }
    all
}

fn file_of(out: &Path, t: &Tile) -> std::path::PathBuf {
    out.join(format!("{}_{}_{}.osm.pbf", t.z, t.x, t.y))
}

/// The live and the peak heap after each step (as the heap probe of the command line).
mod heap {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    struct Counting;
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    /// count only with DUCKOSM_HEAP set (the count slows the threads down)
    pub static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(l) };
            if !p.is_null() && ON.load(Relaxed) {
                PEAK.fetch_max(LIVE.fetch_add(l.size(), Relaxed) + l.size(), Relaxed);
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) };
            if ON.load(Relaxed) {
                LIVE.fetch_sub(l.size(), Relaxed);
            }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let q = unsafe { System.realloc(p, l, new) };
            if !q.is_null() && ON.load(Relaxed) {
                if new >= l.size() {
                    PEAK.fetch_max(LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size(), Relaxed);
                } else {
                    LIVE.fetch_sub(l.size() - new, Relaxed);
                }
            }
            q
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static LAST_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    pub fn report(name: &str) {
        let mib = |b: usize| b as f64 / 1048576.0;
        let live = LIVE.load(Relaxed);
        let now = START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64;
        let secs = (now - LAST_NS.swap(now, Relaxed)) as f64 / 1e9;
        eprintln!("  heap {name:<70} live {:>8.1} MiB   peak {:>8.1} MiB   {secs:>6.3} s", mib(live), mib(PEAK.load(Relaxed)));
        PEAK.store(live, Relaxed);
    }
}

fn main() {
    heap::ON.store(std::env::var_os("DUCKOSM_HEAP").is_some(), std::sync::atomic::Ordering::Relaxed);
    heap::report("start");
    let _ = duckosm_core::util::STEP_HOOK.set(heap::report);
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "compare" {
        let names = |d: &str| {
            let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
            v.sort();
            v
        };
        let (x, y) = (names(&a[2]), names(&a[3]));
        if x != y {
            println!("the files differ: {} and {}", x.len(), y.len());
        }
        let mut bad = 0;
        for n in x.iter().filter(|n| y.contains(n)) {
            let (p, q) = (elements(&Path::new(&a[2]).join(n)), elements(&Path::new(&a[3]).join(n)));
            if p != q {
                bad += 1;
                if bad <= 3 {
                    let ids = |v: &Vec<(i64, Vec<(String, String)>, Vec<(i64, u8, String)>)>| v.iter().map(|r| r.0).collect::<std::collections::BTreeSet<_>>();
                    let (a, b) = (ids(&p.rels), ids(&q.rels));
                    println!("  relations only in A: {:?}, only in B: {:?}", a.difference(&b).take(5).collect::<Vec<_>>(), b.difference(&a).take(5).collect::<Vec<_>>());
                    let ways = |v: &Vec<(i64, Vec<(String, String)>, Vec<i64>)>| v.iter().map(|r| r.0).collect::<std::collections::BTreeSet<_>>();
                    let (a, b) = (ways(&p.ways), ways(&q.ways));
                    println!("  ways only in A: {:?}, only in B: {:?}", a.difference(&b).take(5).collect::<Vec<_>>(), b.difference(&a).take(5).collect::<Vec<_>>());
                }
                println!(
                    "{n}: nodes {} / {}, ways {} / {}, relations {} / {}",
                    p.nodes.len(),
                    q.nodes.len(),
                    p.ways.len(),
                    q.ways.len(),
                    p.rels.len(),
                    q.rels.len()
                );
            }
        }
        println!("{} files, {bad} differ", x.len());
        return;
    }
    let region = a[1].clone();
    let boundary = Boundary::parse(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let z: u8 = a[3].parse().unwrap();
    let out = Path::new(&a[4]);
    std::fs::create_dir_all(out).unwrap();
    let list = tiles(&boundary, z);
    let open = || File::open(&region).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| e.to_string());
    let start = std::time::Instant::now();
    if a.get(5).map(String::as_str) == Some("reference") {
        let mut files = 0;
        // only the tiles of a list (z/x/y,z/x/y,...), when one is given
        let only: Option<Vec<String>> = a.get(6).map(|s| s.split(',').map(str::to_string).collect());
        for t in list.iter().filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.name()))) {
            let path = file_of(out, t);
            let inside = |lon: f64, lat: f64| boundary.contains(lon, lat) && Tile::of(z, lon, lat) == *t;
            let (_, n) = cut_where(open, &inside, BufWriter::new(File::create(&path).unwrap())).unwrap();
            if n.nodes + n.ways + n.relations == 0 || n.nodes == 0 {
                std::fs::remove_file(&path).unwrap();
            } else {
                files += 1;
            }
        }
        println!("{} tiles, {files} files, {:.1} s", list.len(), start.elapsed().as_secs_f64());
    } else {
        let counts = cut_to_tiles(open, &boundary, &list, out, std::env::var_os("NO_MULTIPOLYGONS").is_none()).unwrap();
        let files = counts.iter().filter(|c| c.nodes > 0).count();
        let big = counts.iter().map(|c| c.nodes).max().unwrap_or(0);
        println!("{} tiles, {files} files, the largest {big} nodes, {:.1} s", list.len(), start.elapsed().as_secs_f64());
    }
}
