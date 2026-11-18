//! quack --gdal does not stop when its channels are full. On France by tiles, all threads of the pool waited for the
//! channels of the relations, the tiles waited for the writer, and the writer waited for a job on the same pool: the run
//! stopped with no error. Here: two threads, a channel of one row for the relations, and the tiles of zoom 14 of Monaco
//! (many tiles, each with relations), by tiles and as one file. A run must end in 120 s.
#![cfg(feature = "gpkg")]

fn run_in_time(tiles: Option<duckosm_build::TileSize>, both: bool) {
    let pbf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
    let dir = std::env::temp_dir().join(format!("quack_gdal_full_{}_{}", std::process::id(), tiles.is_some()));
    std::fs::create_dir_all(&dir).unwrap();
    let (gpkg, base) = (dir.join("m.gpkg"), dir.join("m"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = duckosm_build::quack_gdal::quack_gdal(&pbf, Some(&gpkg), both.then_some(base.as_path()), tiles);
        let _ = tx.send(r.map(|n| n.rows));
    });
    let rows = rx
        .recv_timeout(std::time::Duration::from_secs(120))
        .expect("quack --gdal stopped: no end in 120 s")
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(rows[1..], [3960, 3836, 69, 1793, 129]);
}

#[test]
fn full_channels_do_not_stop_the_run() {
    rayon::ThreadPoolBuilder::new().num_threads(2).build_global().unwrap();
    duckosm_core::quack::REL_CHANNEL.store(1, std::sync::atomic::Ordering::Relaxed);
    run_in_time(Some(duckosm_build::TileSize::Zoom(14)), true);
    run_in_time(None, true);
}
