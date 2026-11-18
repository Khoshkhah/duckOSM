//! The planner page's router (`route_map.rs`, `duckosm route-map`): duckosm-planner built for wasm32-unknown-unknown into
//! OUT_DIR. Only with the feature duckdb (the program; the Python module has no route-map). Without the wasm32 target
//! (`rustup target add wasm32-unknown-unknown`) the file is empty and the page says that it has no router.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let file = out.join("duckosm_planner.wasm");
    let ws = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    for d in ["duckosm-planner/src", "duckosm-planner/Cargo.toml", "duckosm-router/src"] {
        println!("cargo:rerun-if-changed={}", ws.join(d).display());
    }
    let mut bytes = Vec::new();
    if std::env::var_os("CARGO_FEATURE_DUCKDB").is_some() {
        // a cargo of its own, in a target folder of its own (the outer cargo holds the lock of its own folder)
        let target = out.join("planner-target");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args(["build", "--release", "--lib", "-p", "duckosm-planner", "--target", "wasm32-unknown-unknown", "--manifest-path"])
            .arg(ws.join("Cargo.toml")).arg("--target-dir").arg(&target);
        for v in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "CARGO_MAKEFLAGS", "MAKEFLAGS"] {
            cmd.env_remove(v);
        }
        match cmd.status() {
            Ok(s) if s.success() => {
                bytes = std::fs::read(target.join("wasm32-unknown-unknown/release/duckosm_planner.wasm")).unwrap_or_default();
            }
            _ => println!("cargo:warning=duckosm-planner was not built for wasm32 (rustup target add wasm32-unknown-unknown): the planner page has no router"),
        }
    }
    std::fs::write(&file, bytes).unwrap();
}
