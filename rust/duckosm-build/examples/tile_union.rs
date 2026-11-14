//! tile_union OUT BOUNDARY MODE TILE_DIR...    the union of the tile builds (output folders named z_x_y) into OUT

use duckosm_build::union::union_tiles;
use duckosm_core::boundary::Boundary;
use duckosm_core::filter::Mode;
use duckosm_core::network::Options;
use duckosm_core::tiles::Tile;
use std::path::PathBuf;

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
    let boundary = Boundary::parse(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let mode = match a[3].as_str() {
        "driving" => Mode::Driving,
        "walking" => Mode::Walking,
        "cycling" => Mode::Cycling,
        m => panic!("no mode {m}"),
    };
    let tiles: Vec<(Tile, PathBuf)> = a[4..]
        .iter()
        .map(|d| {
            let p = PathBuf::from(d);
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let n: Vec<u32> = name.split('_').map(|x| x.parse().unwrap()).collect();
            (Tile { z: n[0] as u8, x: n[1], y: n[2] }, p)
        })
        .collect();
    let t = std::time::Instant::now();
    let st = union_tiles(&tiles, &boundary, mode, &Options::default(), a[1].as_ref()).unwrap();
    println!("{st:?} {:.1} s", t.elapsed().as_secs_f64());
}
