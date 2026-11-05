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

pub fn step(name: &str) {
    if let Some(hook) = STEP_HOOK.get() {
        hook(name);
    }
}
