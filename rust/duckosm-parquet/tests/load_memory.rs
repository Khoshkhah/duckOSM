//! The load step stays in its memory budget (docs/design/streaming_load.md). A counting allocator gives the peak of
//! the heap while `load_raw` reads Monaco with a budget that is smaller than the data of the file.

use duckosm_parquet::{load_raw, RawColumns};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            PEAK.fetch_max(LIVE.fetch_add(l.size(), Relaxed) + l.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                PEAK.fetch_max(
                    LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size(),
                    Relaxed,
                );
            } else {
                LIVE.fetch_sub(l.size() - new, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Monaco 20 times in a row: a `.osm.pbf` file is a list of blocks, so this is a valid input with 20 times the data
/// (the header blocks give no rows).
fn input() -> Vec<u8> {
    let one = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf"),
    )
    .unwrap();
    one.repeat(20)
}

/// The Arrow bytes of all parts of the input: what a load that keeps everything holds at the end.
fn all_parts(data: &[u8]) -> usize {
    let mut sum = 0;
    duckosm_core::pbf::stream_raw(
        data,
        |_, _| false,
        RawColumns::default,
        RawColumns::finish,
        |p, _| {
            sum += p.bytes();
            Ok(())
        },
    )
    .unwrap();
    sum
}

/// The peak of the heap above the start (the input is already in memory), for a load with `budget` bytes. The files
/// go to `io::sink`, so the output does not count.
fn peak(data: &[u8], budget: usize, threads: usize) -> (usize, duckosm_parquet::LoadSummary) {
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let s = load_raw(data, budget, threads, |_| Ok(std::io::sink())).unwrap();
    (PEAK.load(Relaxed) - base, s)
}

#[test]
fn load_stays_in_its_budget() {
    let data = input();
    let all = all_parts(&data);
    for (budget, threads) in [(16 << 20, 1), (16 << 20, 8), (32 << 20, 20), (64 << 20, 20)] {
        let (p, s) = peak(&data, budget, threads);
        println!(
            "budget {:>3} MiB, {threads:>2} threads: peak {:>6} KiB, {} blocks, at most {} in work, ratio {:.1}, all parts {} MiB",
            budget >> 20, p >> 10, s.blocks, s.most_in_flight, s.ratio, all >> 20
        );
        assert_eq!(
            s.rows,
            [20 * 34291, 20 * 5650, 20 * 336],
            "the rows of Monaco 20 times, as ST_READOSM"
        );
        assert!(
            p <= budget,
            "the load used {p} bytes, more than its budget of {budget} bytes"
        );
    }
    // the test means something only when a load that keeps all parts goes far over the budgets
    assert!(
        all > 64 << 20,
        "all parts are {all} bytes: the input is too small for the budgets"
    );
}
