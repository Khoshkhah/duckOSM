//! Level 0 of the outputs: the OSM file (docs/exports/osm-file.md). `duckosm get` finds the boundary of an area, the
//! smallest Geofabrik extract that covers it, downloads the extract to a cache folder and cuts it to the boundary;
//! `duckosm cut --fixes` writes the cut with the OSM fixes in it.
//!
//! The choice of the extract ([`covering`]) is the rule of Studio's page (`web/build_steps/core.js`, `covering`). It
//! has no network and no files, so the browser build can use it too.

use duckosm_core::boundary::Boundary;
use duckosm_core::cutstream::CutCounts;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Geofabrik's list of extracts.
pub const GEOFABRIK_INDEX: &str = "https://download.geofabrik.de/index-v1.json";

/// One extract of Geofabrik's list.
pub struct Extract {
    pub id: String,
    pub name: String,
    /// The address of the `.osm.pbf` file (`<region>-latest.osm.pbf`).
    pub pbf: String,
    pub km2: f64,
    polygon: Boundary,
}

/// The extracts of Geofabrik's list (`index-v1.json`, GeoJSON). A feature with no polygon or no pbf is left out.
pub fn extracts_of(index: &Value) -> Vec<Extract> {
    let mut out = Vec::new();
    for f in index["features"].as_array().into_iter().flatten() {
        let (p, g) = (&f["properties"], &f["geometry"]);
        let (Some(pbf), Ok(polygon)) = (p["urls"]["pbf"].as_str(), Boundary::from_geojson(g)) else {
            continue;
        };
        let text = |k: &str| p[k].as_str().unwrap_or_default().to_string();
        out.push(Extract { id: text("id"), name: text("name"), pbf: pbf.to_string(), km2: area_km2(g), polygon });
    }
    out
}

/// The extracts that contain all points of the outer rings of `area` (at most 2000 are tested), the smallest first.
/// The polygons of Geofabrik's list are about 1 km larger than the extracts, so an area at most 1 km outside is found.
pub fn covering<'a>(extracts: &'a [Extract], area: &Boundary) -> Vec<&'a Extract> {
    let pts: Vec<(f64, f64)> = area.outer_rings().flatten().copied().collect();
    let step = pts.len().div_ceil(2000).max(1);
    let some: Vec<(f64, f64)> = pts.into_iter().step_by(step).collect();
    let mut out: Vec<&Extract> = extracts.iter().filter(|x| some.iter().all(|&(lon, lat)| x.polygon.contains(lon, lat))).collect();
    out.sort_by(|a, b| a.km2.total_cmp(&b.km2));
    out
}

/// The area in square kilometres of a GeoJSON Polygon or MultiPolygon (a sphere of radius 6378137 m; the holes are
/// taken away), as `areaKm2` of Studio's page.
fn area_km2(g: &Value) -> f64 {
    let ring = |r: &Value| {
        let p: Vec<(f64, f64)> = r.as_array().into_iter().flatten().filter_map(|c| Some((c[0].as_f64()?, c[1].as_f64()?))).collect();
        let a: f64 = p.windows(2).map(|w| (w[1].0 - w[0].0).to_radians() * (2.0 + w[0].1.to_radians().sin() + w[1].1.to_radians().sin())).sum();
        (a * 6378137.0 * 6378137.0 / 2.0).abs()
    };
    let polygon = |p: &Value| {
        let rings = p.as_array().map(Vec::as_slice).unwrap_or_default();
        rings.first().map_or(0.0, ring) - rings.iter().skip(1).map(ring).sum::<f64>()
    };
    let km2 = match g["type"].as_str() {
        Some("Polygon") => polygon(&g["coordinates"]),
        Some("MultiPolygon") => g["coordinates"].as_array().into_iter().flatten().map(polygon).sum(),
        _ => 0.0,
    };
    km2 / 1e6
}

/// Stop a boundary that is not in WGS 84 longitude and latitude: a position outside -180..180, -90..90, or a GeoJSON
/// `crs` member that names another system (docs/reference/boundary.md).
pub fn check_boundary(text: &str) -> Result<Boundary, String> {
    let b = Boundary::parse(text)?;
    let (w, s, e, n) = b.bbox();
    if w < -180.0 || e > 180.0 || s < -90.0 || n > 90.0 {
        return Err(format!(
            "boundary: the box {w}, {s}, {e}, {n} is not longitude and latitude in degrees (WGS 84, EPSG:4326): see docs/reference/boundary.md"
        ));
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        open_ring(&v)?;
        if let Some(crs) = v.pointer("/crs/properties/name").and_then(Value::as_str) {
            if !(crs.ends_with("CRS84") || crs.ends_with("4326")) {
                return Err(format!("boundary: the crs is {crs}; the boundary must be in WGS 84 longitude and latitude (EPSG:4326)"));
            }
        }
    }
    Ok(b)
}

/// Stop a GeoJSON ring that is not closed (RFC 7946: the last position equals the first, 4 positions or more). The
/// cut closes it itself, but GDAL and DuckDB can refuse it in the build.
fn open_ring(v: &Value) -> Result<(), String> {
    let polygon = |p: &Value| -> Result<(), String> {
        for r in p.as_array().into_iter().flatten() {
            let pts = r.as_array().map(Vec::as_slice).unwrap_or_default();
            if pts.len() < 4 || pts.first() != pts.last() {
                return Err(format!(
                    "boundary: a ring is not closed ({} positions, the first {}, the last {}): add the first position again at the end",
                    pts.len(),
                    pts.first().unwrap_or(&Value::Null),
                    pts.last().unwrap_or(&Value::Null)
                ));
            }
        }
        Ok(())
    };
    match v["type"].as_str() {
        Some("FeatureCollection") => v["features"].as_array().into_iter().flatten().try_for_each(open_ring),
        Some("Feature") => open_ring(&v["geometry"]),
        Some("Polygon") => polygon(&v["coordinates"]),
        Some("MultiPolygon") => v["coordinates"].as_array().into_iter().flatten().try_for_each(polygon),
        _ => Ok(()),
    }
}

/// Cut the region file `region` to `boundary` (text) and write the area to `out` (`duckosm cut`). With `fixes` (the
/// fixes file of `duckosm build --fixes`), the area file has the fixes in it: the cut goes to `<out>.cut` first, then
/// [`duckosm_core::fixpbf::write_fixed`] copies it to `out` with the fixes.
pub fn cut_with_fixes(region: &Path, boundary: &str, out: &Path, fixes: Option<&Path>) -> Result<CutCounts, String> {
    check_boundary(boundary)?;
    let Some(fixes) = fixes else {
        return crate::cut_file(region, boundary, out);
    };
    let osm = crate::config::read_osm_fixes(fixes)?;
    let cut = PathBuf::from(format!("{}.cut", out.display()));
    let res = (|| {
        crate::cut_file(region, boundary, &cut)?;
        let open = || {
            duckosm_core::fs::File::open(&cut).map(|f| std::io::BufReader::with_capacity(1 << 20, f)).map_err(|e| format!("{}: {e}", cut.display()))
        };
        let q = duckosm_core::quack::QuackFixes::new(&osm, open)?;
        let f = duckosm_core::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let (w, n) = duckosm_core::fixpbf::write_fixed(open, &q, &osm.turn_restrictions, std::io::BufWriter::with_capacity(1 << 20, f))?;
        w.into_inner().map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(n)
    })();
    let _ = std::fs::remove_file(&cut);
    res
}

#[cfg(feature = "get")]
pub use download::{cache_dir, download, region_file};

#[cfg(feature = "get")]
mod download {
    use super::*;
    use md5::{Digest, Md5};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// The cache folder of the downloads: `$DUCKOSM_CACHE`, else `$XDG_CACHE_HOME/duckosm`, else `~/.cache/duckosm`.
    pub fn cache_dir() -> PathBuf {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        env("DUCKOSM_CACHE")
            .or_else(|| env("XDG_CACHE_HOME").map(|d| d.join("duckosm")))
            .or_else(|| env("HOME").or_else(|| env("USERPROFILE")).map(|h| h.join(".cache").join("duckosm")))
            .unwrap_or_else(|| PathBuf::from(".duckosm-cache"))
    }

    /// The date `back` days ago, UTC, as Geofabrik writes it in the name of a dated file: yymmdd.
    fn yymmdd(back: u64) -> String {
        let days = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) / 86400).saturating_sub(back) as i64;
        // the civil date from the days since 1970-01-01 (Howard Hinnant's algorithm)
        let z = days + 719468;
        let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let (doy, y) = (doe - (365 * yoe + yoe / 4 - yoe / 100), yoe + era * 400);
        let mp = (5 * doy + 2) / 153;
        let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
        format!("{:02}{:02}{:02}", (if m <= 2 { y + 1 } else { y }) % 100, m, d)
    }

    /// The dated file that `<region>-latest.osm.pbf` is now, and its size. "latest" is a redirect that Geofabrik answers
    /// after about 30 s, but a dated file that is there answers in 0.5 s (Studio does the same, `www/app/regions.js`):
    /// ask for the files of the last 5 days (UTC) at the same time and take the newest. Else "latest" itself.
    fn resolve(url: &str) -> Result<(String, u64), String> {
        use ureq::ResponseExt;
        let size = |h: &ureq::http::Response<ureq::Body>| {
            h.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0u64)
        };
        if url.contains("-latest.") {
            let quick = ureq::Agent::new_with_config(
                ureq::config::Config::builder().timeout_global(Some(std::time::Duration::from_millis(1500))).http_status_as_error(false).build(),
            );
            let tries: Vec<_> = (0..5)
                .map(|back| {
                    let (agent, dated) = (quick.clone(), url.replacen("-latest.", &format!("-{}.", yymmdd(back)), 1));
                    std::thread::spawn(move || agent.head(&dated).call().ok().filter(|h| h.status().as_u16() == 200).map(|h| (dated, h)))
                })
                .collect();
            for t in tries {
                if let Some((dated, h)) = t.join().ok().flatten() {
                    return Ok((dated, size(&h)));
                }
            }
        }
        let h = ureq::head(url).call().map_err(|e| format!("{url}: {e}"))?;
        Ok((h.get_uri().to_string(), size(&h)))
    }

    /// The bytes `from..to` of `url` appended to the file `part` (it holds the bytes from `from` that came before:
    /// the download goes on from there). A failure is tried again 3 times.
    fn fetch(url: &str, part: &Path, from: u64, to: u64, got: &AtomicU64) -> Result<(), String> {
        let mut tries = 0;
        loop {
            let have = std::fs::metadata(part).map_or(0, |m| m.len());
            if from + have >= to {
                return Ok(());
            }
            let one = || -> Result<(), String> {
                let mut resp = ureq::get(url).header("Range", format!("bytes={}-{}", from + have, to - 1)).call().map_err(|e| format!("{url}: {e}"))?;
                if resp.status().as_u16() != 206 {
                    return Err(format!("{url}: the server did not give the range (HTTP {})", resp.status()));
                }
                let mut file = std::fs::OpenOptions::new().create(true).append(true).open(part).map_err(|e| format!("{}: {e}", part.display()))?;
                let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
                let mut buf = vec![0u8; 1 << 16];
                loop {
                    let n = reader.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
                    if n == 0 {
                        return Ok(());
                    }
                    file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
                    got.fetch_add(n as u64, Relaxed);
                }
            };
            if let Err(e) = one() {
                tries += 1;
                if tries > 3 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(500 * tries));
            }
        }
    }

    /// Download `url` to `to`, with the progress on the standard error, and check it with the `.md5` file next to it.
    /// The download is in parts (`<to>.part<i>`, 8 connections for a file of 16 MB or more: Geofabrik gives about
    /// 0.5 MB/s to one connection). A download that stopped goes on from the bytes of its parts, when the dated file is
    /// the same (`<to>.url`). Gives the dated address.
    pub fn download(url: &str, to: &Path) -> Result<String, String> {
        let (url, total) = resolve(url)?;
        if total == 0 {
            return Err(format!("{url}: the server gives no size"));
        }
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let part = |i: u64| PathBuf::from(format!("{}.part{i}", to.display()));
        let mark = PathBuf::from(format!("{}.url", to.display()));
        let n = if total >= 16 << 20 { 8 } else { 1 };
        // the parts of another version of the file are not used
        if std::fs::read_to_string(&mark).ok().as_deref() != Some(url.as_str()) {
            (0..8).for_each(|i| { let _ = std::fs::remove_file(part(i)); });
            std::fs::write(&mark, &url).map_err(|e| format!("{}: {e}", mark.display()))?;
        }
        let step = total.div_ceil(n);
        let got = AtomicU64::new((0..n).map(|i| std::fs::metadata(part(i)).map_or(0, |m| m.len())).sum());
        let done = std::sync::atomic::AtomicBool::new(false);
        let mb = |b: u64| b as f64 / 1048576.0;
        std::thread::scope(|s| {
            s.spawn(|| {
                let (start, at0) = (std::time::Instant::now(), got.load(Relaxed));
                while !done.load(Relaxed) {
                    let g = got.load(Relaxed);
                    let speed = mb(g - at0) / start.elapsed().as_secs_f64().max(0.1);
                    eprint!("\r  download {:.1} of {:.1} MB ({speed:.1} MB/s)   ", mb(g), mb(total));
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            });
            let res = std::thread::scope(|p| {
                let h: Vec<_> = (0..n).map(|i| p.spawn({ let (url, part, got) = (&url, part(i), &got); move || fetch(url, &part, i * step, ((i + 1) * step).min(total), got) })).collect();
                h.into_iter().try_for_each(|h| h.join().map_err(|_| "a download thread stopped".to_string())?)
            });
            done.store(true, Relaxed);
            res
        })?;
        eprintln!("\r  download {:.1} MB: done{:>30}", mb(total), "");
        // the parts into one file, and its MD5 sum
        let tmp = PathBuf::from(format!("{}.tmp", to.display()));
        let mut out = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let mut sum = Md5::new();
        let mut buf = vec![0u8; 1 << 20];
        for i in 0..n {
            let mut f = std::fs::File::open(part(i)).map_err(|e| format!("{}: {e}", part(i).display()))?;
            loop {
                let k = f.read(&mut buf).map_err(|e| e.to_string())?;
                if k == 0 {
                    break;
                }
                sum.update(&buf[..k]);
                out.write_all(&buf[..k]).map_err(|e| format!("{}: {e}", tmp.display()))?;
            }
        }
        drop(out);
        let len = std::fs::metadata(&tmp).map_or(0, |m| m.len());
        let md5: String = sum.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let clean = || {
            (0..n).for_each(|i| { let _ = std::fs::remove_file(part(i)); });
            let _ = std::fs::remove_file(&mark);
        };
        if len != total {
            clean();
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{url}: got {len} of {total} bytes; run the command again"));
        }
        match ureq::get(&format!("{url}.md5")).call().and_then(|mut r| r.body_mut().read_to_string()) {
            Ok(text) if text.split_whitespace().next() == Some(md5.as_str()) => eprintln!("  md5 {md5}: correct"),
            Ok(text) => {
                clean();
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("{url}: the MD5 sum is {md5}, Geofabrik gives {}; run the command again", text.trim()));
            }
            Err(e) => eprintln!("  warning: no MD5 sum to check ({url}.md5: {e})"),
        }
        std::fs::rename(&tmp, to).map_err(|e| format!("{}: {e}", to.display()))?;
        clean();
        Ok(url)
    }

    /// The region file for `area`: the smallest Geofabrik extract that covers it, in the cache folder `cache` (downloaded
    /// when it is not there, or with `refresh`). `index`: the list of extracts (an address or a file; None: Geofabrik's,
    /// kept in the cache for 7 days). Gives the file and the extract's name.
    pub fn region_file(area: &Boundary, cache: &Path, index: Option<&str>, refresh: bool) -> Result<(PathBuf, String), String> {
        let text = match index {
            Some(i) if !i.starts_with("http") => std::fs::read_to_string(i).map_err(|e| format!("{i}: {e}"))?,
            _ => {
                let url = index.unwrap_or(GEOFABRIK_INDEX);
                let file = cache.join("index-v1.json");
                let age = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
                if index.is_some() || refresh || age.is_none_or(|a| a.as_secs() > 7 * 86400) {
                    eprintln!("  the list of extracts: {url}");
                    let t = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?.body_mut().with_config().limit(256 << 20).read_to_string().map_err(|e| format!("{url}: {e}"))?;
                    std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
                    std::fs::write(&file, &t).map_err(|e| format!("{}: {e}", file.display()))?;
                }
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?
            }
        };
        let index: Value = serde_json::from_str(&text).map_err(|e| format!("the list of extracts: {e}"))?;
        let all = extracts_of(&index);
        let x = *covering(&all, area).first().ok_or("no Geofabrik extract covers the boundary")?;
        // <cache>/geofabrik/<the path of the address>, for example geofabrik/europe/monaco-latest.osm.pbf
        let rel = x.pbf.split_once("://").map_or(x.pbf.as_str(), |u| u.1).split_once('/').map_or("", |u| u.1);
        if rel.is_empty() || rel.split('/').any(|p| p.is_empty() || p == "..") {
            return Err(format!("{}: not a file address", x.pbf));
        }
        let file = cache.join("geofabrik").join(rel);
        eprintln!("  extract: {} ({}, {:.0} km²)", x.name, x.id, x.km2);
        if file.is_file() && !refresh {
            eprintln!("  in the cache: {}", file.display());
        } else {
            let dated = download(&x.pbf, &file)?;
            eprintln!("  saved {} ({dated})", file.display());
        }
        Ok((file, x.name.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest extract that holds all points of the area is first; an extract that holds only a part is left out.
    #[test]
    fn the_smallest_extract_that_covers_the_area() {
        let sq = |w: f64, s: f64, e: f64, n: f64| serde_json::json!({"type": "MultiPolygon", "coordinates": [[[[w, s], [e, s], [e, n], [w, n], [w, s]]]]});
        let f = |id: &str, g: Value| serde_json::json!({"type": "Feature", "properties": {"id": id, "name": id, "urls": {"pbf": format!("https://x/{id}-latest.osm.pbf")}}, "geometry": g});
        let index = serde_json::json!({"features": [
            f("europe", sq(-10.0, 35.0, 30.0, 70.0)),
            f("france", sq(-5.0, 42.0, 8.0, 51.0)),
            f("monaco", sq(7.40, 43.72, 7.44, 43.76)),
            {"type": "Feature", "properties": {"id": "nofile"}, "geometry": sq(7.0, 43.0, 8.0, 44.0)},
        ]});
        let all = extracts_of(&index);
        assert_eq!(all.len(), 3);
        let ids = |b: &str| covering(&all, &Boundary::parse(b).unwrap()).iter().map(|x| x.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("7.41,43.73,7.43,43.75"), ["monaco", "france", "europe"]);
        assert_eq!(ids("7.0,43.0,7.5,43.5"), ["france", "europe"]);
        assert!(ids("100,0,101,1").is_empty());
        // a 1° square at the equator is about 12391 km² on the sphere of radius 6378137 m
        assert!((area_km2(&sq(0.0, 0.0, 1.0, 1.0)) - 12391.0).abs() < 1.0);
    }

    /// `cut --fixes` on the Monaco sample: the file is sorted by type, then id; a removed way is not there; the tags,
    /// the network rules, the new ways with their new nodes and the turn rule are there.
    #[test]
    fn the_cut_has_the_fixes() {
        use duckosm_core::graph::MemberType;
        use duckosm_core::raw::RawSink;
        #[derive(Default)]
        struct All {
            nodes: Vec<(i64, Vec<(String, String)>)>,
            ways: Vec<(i64, Vec<(String, String)>, Vec<i64>)>,
            rels: Vec<(i64, Vec<(String, String)>, Vec<(i64, String)>)>,
        }
        impl RawSink for All {
            fn node<'a>(&mut self, id: i64, _: f64, _: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
                self.nodes.push((id, t.map(|(k, v)| (k.into(), v.into())).collect()));
            }
            fn way<'a>(&mut self, id: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, r: &mut dyn Iterator<Item = i64>) {
                self.ways.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), r.collect()));
            }
            fn relation<'a>(&mut self, id: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, m: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>) {
                self.rels.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), m.map(|(i, _, r)| (i, r.to_string())).collect()));
            }
        }
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
        let boundary = std::fs::read_to_string(data.join("monaco.geojson")).unwrap();
        let dir = std::env::temp_dir().join(format!("duckosm-cut-fixes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let read = |f: &Path| {
            let (mut all, mut buf, mut r) = (All::default(), Default::default(), std::fs::File::open(f).unwrap());
            while let Some(b) = duckosm_core::pbfraw::read_block(&mut r).unwrap() {
                duckosm_core::pbfraw::decode_block(&b, [true; 3], &mut buf, &mut all).unwrap();
            }
            all
        };
        let tag = |t: &[(String, String)], k: &str| t.iter().find(|x| x.0 == k).map(|x| x.1.clone());
        for fixes in ["page_fixes/osm_fixes.yaml", "osm_fixes_quack.yaml"] {
            let out = dir.join("cut.osm.pbf");
            let n = cut_with_fixes(&data.join("monaco_sample.osm.pbf"), &boundary, &out, Some(&data.join(fixes))).unwrap();
            let a = read(&out);
            assert_eq!((n.nodes, n.ways, n.relations), (a.nodes.len(), a.ways.len(), a.rels.len()), "{fixes}");
            assert!(a.nodes.windows(2).all(|w| w[0].0 < w[1].0) && a.ways.windows(2).all(|w| w[0].0 < w[1].0) && a.rels.windows(2).all(|w| w[0].0 < w[1].0));
            assert!(duckosm_core::cutstream::header(std::fs::File::open(&out).unwrap()).unwrap().1, "sorted in the header");
            let way = |id: i64| a.ways.iter().find(|w| w.0 == id);
            let new = way(9_000_000_000_000 + (fixes == "osm_fixes_quack.yaml") as i64 * 2).expect("the new way");
            assert_eq!(new.2, [21911886, new.0 * 1000 + 2, 21914340]);
            assert_eq!(tag(&new.1, "highway").as_deref(), Some("service"));
            assert!(a.nodes.iter().any(|x| x.0 == new.0 * 1000 + 2), "the new node");
            let w = way(503462470).unwrap();
            assert_eq!((tag(&w.1, "oneway").as_deref(), tag(&w.1, "lanes").as_deref()), (Some("yes"), Some("3")));
            if fixes == "osm_fixes_quack.yaml" {
                assert!(way(23715051).is_none(), "removed");
                assert_eq!(way(25722685).unwrap().2.first(), Some(&280486998), "keep_nodes");
                assert_eq!(tag(&a.nodes.iter().find(|x| x.0 == 21911886).unwrap().1, "crossing").as_deref(), Some("marked"));
                let r = a.rels.iter().find(|r| r.0 == 393226).unwrap();
                assert_eq!(tag(&r.1, "name").as_deref(), Some("Test relation"));
            } else {
                assert!(way(25722685).is_none(), "removed");
                let r = &a.rels[0];
                assert_eq!((r.0, tag(&r.1, "restriction").as_deref()), (-1, Some("no_left_turn")));
                assert_eq!(r.2, [(4097656, "from".to_string()), (21912089, "via".into()), (35092475, "to".into())]);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_boundary_must_be_longitude_and_latitude() {
        assert!(check_boundary("7.40,43.72,7.44,43.76").is_ok());
        assert!(check_boundary(r#"{"type":"Polygon","coordinates":[[[700000,4800000],[700100,4800000],[700100,4800100],[700000,4800000]]]}"#).is_err());
        let crs = r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::3006"}},"features":[{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[7,43],[8,43],[8,44],[7,43]]]}}]}"#;
        assert!(check_boundary(crs).err().unwrap().contains("EPSG::3006"));
        assert!(check_boundary(&crs.replace("EPSG::3006", "OGC:1.3:CRS84")).is_ok());
        let open = r#"{"type":"Polygon","coordinates":[[[7,43],[8,43],[8,44]]]}"#;
        assert!(check_boundary(open).err().unwrap().contains("not closed"));
    }
}
