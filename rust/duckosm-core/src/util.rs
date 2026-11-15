//! A small fast hasher for integer keys (the "Fx" hash). Node ids are not under attack, so this is safe here.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

const K: u64 = 0x517c_c1b7_2722_0a95;

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(K);
        }
    }
    fn write_i64(&mut self, i: i64) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(K);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(K);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type FxHashSet<K> = std::collections::HashSet<K, BuildHasherDefault<FxHasher>>;

/// A hook for the memory probe (`examples/mem_probe.rs`): the network step calls `step` at the start of each numbered step. Without a
/// probe it costs one read of a static.
pub static STEP_HOOK: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

/// Raise the limit of open files of this process to 10 240 (or the hard limit), one time. The tiles of quack run at the
/// same time, each with about 170 temporary files open (the buckets of the node ids and of the points), and macOS gives a
/// process 256. Elsewhere it does nothing.
pub fn raise_open_files() {
    #[cfg(unix)]
    {
        static DONE: std::sync::Once = std::sync::Once::new();
        // SAFETY: getrlimit and setrlimit with a valid struct; a failure only keeps the old limit
        DONE.call_once(|| unsafe {
            let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 {
                let want = r.rlim_max.min(10_240);
                if r.rlim_cur < want {
                    r.rlim_cur = want;
                    let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &r);
                }
            }
        });
    }
}

thread_local! {
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with no [`step`] on this thread (a tile that runs on a helper thread: in the browser the step hook posts to
/// the page from the thread of the run only).
pub fn quiet<T>(f: impl FnOnce() -> T) -> T {
    let old = QUIET.with(|q| q.replace(true));
    let r = f();
    QUIET.with(|q| q.set(old));
    r
}

pub fn step(name: &str) {
    if QUIET.with(|q| q.get()) {
        return;
    }
    if let Some(hook) = STEP_HOOK.get() {
        hook(name);
    }
}

/// Give the memory that the allocator keeps back to the system (glibc: `malloc_trim`, all arenas): between the passes of a
/// step, after many threads made and freed many small blocks (read 2 of the tile cut). Nothing elsewhere.
pub fn trim_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe { malloc_trim(0) };
    }
}
