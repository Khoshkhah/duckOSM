//! A measurement of the sorted join of quack by tiles: quack_join_bench FILE.osm.pbf ZOOM
use std::time::Instant;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&a[1]);
    let zoom: u8 = a[2].parse().unwrap();
    let open = || Ok(std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?));
    let tiles = duckosm_core::quack::tiles_of_file(&open, duckosm_core::quack::TileSize::Zoom(zoom)).unwrap();
    let world = duckosm_core::boundary::Boundary::parse(duckosm_core::quack::WORLD).unwrap();
    let mut files: Vec<Vec<u8>> = vec![Vec::new(); tiles.len()];
    duckosm_core::tilecut::cut_to_sink(open, &world, &tiles, &["multipolygon", "boundary"], false, &mut |t, b| {
        files[t as usize].extend_from_slice(b);
        Ok(())
    })
    .unwrap();
    duckosm_core::util::raise_open_files();
    for sort in [false, true] {
        let t = Instant::now();
        for (i, f) in files.iter().enumerate().filter(|(_, f)| !f.is_empty()) {
            let mut p = Vec::new();
            duckosm_build::quack_tile_to(|| Ok(std::io::Cursor::new(&f[..])), tiles[i], &mut p, sort, None).unwrap();
        }
        eprintln!("tiles, sort {sort}: {:.1} s", t.elapsed().as_secs_f64());
    }
    let t = Instant::now();
    let parts: Vec<Vec<u8>> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_empty())
        .map(|(i, f)| {
            let mut p = Vec::new();
            duckosm_build::quack_tile_to(|| Ok(std::io::Cursor::new(&f[..])), tiles[i], &mut p, true, None).unwrap();
            p
        })
        .collect();
    eprintln!("tiles sorted: {:.1} s, {} parts", t.elapsed().as_secs_f64(), parts.len());
    let cur = || parts.iter().map(|p| std::io::Cursor::new(p.clone())).collect::<Vec<_>>();
    // the merge alone: decode of the parts, the heap
    let t = Instant::now();
    let mut ps: Vec<_> = cur().into_iter().map(|c| duckosm_parquet::quack::SortedPart::open(c).unwrap()).collect();
    let mut heap: std::collections::BinaryHeap<_> = ps.iter().enumerate().filter_map(|(i, p)| p.key().map(|k| std::cmp::Reverse((k, i)))).collect();
    let mut n = 0;
    let mut row = Vec::new();
    while let Some(std::cmp::Reverse((_, i))) = heap.pop() {
        row.clear();
        ps[i].put(n, &mut row).unwrap();
        if let Some(k) = ps[i].key() {
            heap.push(std::cmp::Reverse((k, i)));
        }
        n += 1;
    }
    eprintln!("merge alone: {:.1} s, {n} rows", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let mut out = Vec::new();
    let n = duckosm_build::quack_join_to(cur(), &mut out, true, &mut |_, _| {}).unwrap();
    eprintln!("sorted join: {:.1} s, {n} rows, {} MB", t.elapsed().as_secs_f64(), out.len() >> 20);
    // the design before: all rows of the parts into StrSort (runs), then its strips
    let t = Instant::now();
    let mut s = duckosm_core::quack::StrSort::default();
    for p in &parts {
        duckosm_parquet::quack::read_quack_bytes(p.clone(), |b| s.push(duckosm_parquet::quack::features_of(&b)?)).unwrap();
    }
    eprintln!("before, push: {:.1} s", t.elapsed().as_secs_f64());
    let mut w = duckosm_parquet::quack::QuackWriter::new(Vec::new()).unwrap();
    s.finish(|rows| w.write(&duckosm_parquet::quack::batch(&rows)?)).unwrap();
    w.close().unwrap();
    eprintln!("before, all: {:.1} s", t.elapsed().as_secs_f64());
}
