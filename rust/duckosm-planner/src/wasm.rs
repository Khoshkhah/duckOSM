//! The exports of the WebAssembly module for the planner page (templates/planner.html). Text goes in and out as UTF-8 in the
//! module's memory: the page gets a buffer with `rm_alloc`, writes the JSON into it and calls `rm_load` (the page's data) or
//! `rm_plan` (a request, `Area::plan`). The answer is at `rm_out_ptr`, `rm_out_len` bytes long, until the next call.

use crate::Area;
use serde_json::{json, Value};
use std::cell::RefCell;

thread_local! {
    static AREA: RefCell<Option<Area>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[no_mangle]
pub extern "C" fn rm_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` and `n` are a buffer of `rm_alloc`.
#[no_mangle]
pub unsafe extern "C" fn rm_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n));
}

/// # Safety
/// `p` and `n` are a buffer of `rm_alloc` with `n` bytes written.
unsafe fn input(p: *mut u8, n: usize) -> Result<Value, String> {
    let v = Vec::from_raw_parts(p, n, n);
    serde_json::from_slice(&v).map_err(|e| e.to_string())
}

fn answer(v: Value) -> usize {
    OUT.with(|o| {
        *o.borrow_mut() = v.to_string().into_bytes();
        o.borrow().len()
    })
}

/// Load the page's data (the buffer is freed). The answer: `{}`, or `{error}`.
///
/// # Safety
/// `p` and `n` are a buffer of `rm_alloc` with `n` bytes written.
#[no_mangle]
pub unsafe extern "C" fn rm_load(p: *mut u8, n: usize) -> usize {
    let r = input(p, n).and_then(|d| Area::from_data(&d));
    answer(match r {
        Ok(a) => {
            AREA.with(|x| *x.borrow_mut() = Some(a));
            json!({})
        }
        Err(e) => json!({"error": e}),
    })
}

/// Answer a request (`Area::plan`; the buffer is freed). An error is `{error}`.
///
/// # Safety
/// `p` and `n` are a buffer of `rm_alloc` with `n` bytes written.
#[no_mangle]
pub unsafe extern "C" fn rm_plan(p: *mut u8, n: usize) -> usize {
    let q = input(p, n);
    answer(AREA.with(|a| match (a.borrow().as_ref(), q) {
        (Some(a), Ok(q)) => a.plan(&q).unwrap_or_else(|e| json!({"error": e})),
        (None, _) => json!({"error": "no data: rm_load first"}),
        (_, Err(e)) => json!({"error": e}),
    }))
}

#[no_mangle]
pub extern "C" fn rm_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}
