//! The raw cut as a file: `.osm.pbf` (write_pbf) and `.osm` XML (write_xml). The page gives the user the area
//! that it cut from a region file, as osmium's `smart` cut gives it (raw.rs), so that other tools can use it.
//!
//! The PBF has the header block and then blocks of 8000 nodes (dense), ways or relations, each in a zlib blob. The ids
//! are sorted in each type, so the header says `Sort.Type_then_ID`. A node has its coordinates in 1e-7 degrees (the
//! granularity of the standard); the files of Geofabrik have the same. There is no metadata (version, time, user).
//! The bytes do not depend on the target: `miniz_oxide` is pure Rust. The browser build and the native build write the
//! same file.

use crate::graph::MemberType;
use crate::raw::{Raw, RawTags};
use crate::util::FxHashMap;
use std::io::Write;

/// Entities in one block of the PBF (the number that osmium uses).
const BLOCK: usize = 8000;

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// A field of wire type 2 (length-delimited).
fn bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    varint(out, (field << 3 | 2) as u64);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// A field of wire type 0 (varint).
fn int(out: &mut Vec<u8>, field: u32, v: i64) {
    varint(out, (field << 3) as u64);
    varint(out, v as u64);
}

fn packed(out: &mut Vec<u8>, field: u32, vals: impl IntoIterator<Item = u64>) {
    let mut b = Vec::new();
    for v in vals {
        varint(&mut b, v);
    }
    if !b.is_empty() {
        bytes(out, field, &b);
    }
}

/// The string table of one block. Index 0 is the empty string, as the standard asks.
struct Strings<'a> {
    list: Vec<&'a str>,
    index: FxHashMap<&'a str, u64>,
}

impl<'a> Strings<'a> {
    fn new() -> Self {
        let mut index = FxHashMap::default();
        index.insert("", 0);
        Strings {
            list: vec![""],
            index,
        }
    }

    fn id(&mut self, s: &'a str) -> u64 {
        let list = &mut self.list;
        *self.index.entry(s).or_insert_with(|| {
            list.push(s);
            list.len() as u64 - 1
        })
    }

    /// The table, then the group, as a `PrimitiveBlock`.
    fn block(&self, group: &[u8]) -> Vec<u8> {
        let mut table = Vec::new();
        for s in &self.list {
            bytes(&mut table, 1, s.as_bytes());
        }
        let mut out = Vec::new();
        bytes(&mut out, 1, &table);
        bytes(&mut out, 2, group);
        out
    }

    fn tags(&mut self, tags: &'a RawTags, keys: &mut Vec<u64>, vals: &mut Vec<u64>) {
        for (k, v) in tags {
            keys.push(self.id(k));
            vals.push(self.id(v));
        }
    }
}

fn file_blob<W: Write>(w: &mut W, kind: &str, data: &[u8]) -> Result<(), String> {
    let mut blob = Vec::new();
    int(&mut blob, 2, data.len() as i64);
    bytes(
        &mut blob,
        3,
        &miniz_oxide::deflate::compress_to_vec_zlib(data, 6),
    );
    let mut head = Vec::new();
    bytes(&mut head, 1, kind.as_bytes());
    int(&mut head, 3, blob.len() as i64);
    let e = |e: std::io::Error| e.to_string();
    w.write_all(&(head.len() as u32).to_be_bytes()).map_err(e)?;
    w.write_all(&head).map_err(e)?;
    w.write_all(&blob).map_err(e)
}

/// Write the cut as a `.osm.pbf` file. `timestamp` (seconds since 1970) is the replication time of the region
/// file's header, when it has one.
pub fn write_pbf<W: Write>(raw: &Raw, timestamp: Option<i64>, w: &mut W) -> Result<(), String> {
    let mut head = Vec::new();
    bytes(&mut head, 4, b"OsmSchema-V0.6");
    bytes(&mut head, 4, b"DenseNodes");
    bytes(&mut head, 5, b"Sort.Type_then_ID");
    bytes(
        &mut head,
        16,
        concat!("duckosm-rust ", env!("CARGO_PKG_VERSION")).as_bytes(),
    );
    if let Some(t) = timestamp {
        int(&mut head, 32, t);
    }
    file_blob(w, "OSMHeader", &head)?;

    for chunk in raw.nodes.chunks(BLOCK) {
        let mut s = Strings::new();
        let (mut ids, mut lats, mut lons, mut kv) = (vec![], vec![], vec![], vec![]);
        let (mut id0, mut lat0, mut lon0) = (0, 0, 0);
        for n in chunk {
            let lat = (n.lat * 1e7).round() as i64;
            let lon = (n.lon * 1e7).round() as i64;
            ids.push(zigzag(n.id - id0));
            lats.push(zigzag(lat - lat0));
            lons.push(zigzag(lon - lon0));
            (id0, lat0, lon0) = (n.id, lat, lon);
            for (k, v) in &n.tags {
                kv.extend([s.id(k), s.id(v)]);
            }
            kv.push(0);
        }
        let mut dense = Vec::new();
        packed(&mut dense, 1, ids);
        packed(&mut dense, 8, lats);
        packed(&mut dense, 9, lons);
        // no tags at all: the standard allows to leave `keys_vals` out
        if chunk.iter().any(|n| !n.tags.is_empty()) {
            packed(&mut dense, 10, kv);
        }
        let mut group = Vec::new();
        bytes(&mut group, 2, &dense);
        file_blob(w, "OSMData", &s.block(&group))?;
    }

    for chunk in raw.ways.chunks(BLOCK) {
        let mut s = Strings::new();
        let mut group = Vec::new();
        for way in chunk {
            let (mut k, mut v) = (vec![], vec![]);
            s.tags(&way.tags, &mut k, &mut v);
            let mut m = Vec::new();
            int(&mut m, 1, way.id);
            packed(&mut m, 2, k);
            packed(&mut m, 3, v);
            let mut prev = 0;
            packed(
                &mut m,
                8,
                way.refs.iter().map(|&r| {
                    let d = zigzag(r - prev);
                    prev = r;
                    d
                }),
            );
            bytes(&mut group, 3, &m);
        }
        file_blob(w, "OSMData", &s.block(&group))?;
    }

    for chunk in raw.relations.chunks(BLOCK) {
        let mut s = Strings::new();
        let mut group = Vec::new();
        for rel in chunk {
            let (mut k, mut v) = (vec![], vec![]);
            s.tags(&rel.tags, &mut k, &mut v);
            let mut m = Vec::new();
            int(&mut m, 1, rel.id);
            packed(&mut m, 2, k);
            packed(&mut m, 3, v);
            packed(&mut m, 8, rel.members.iter().map(|(_, _, role)| s.id(role)));
            let mut prev = 0;
            packed(
                &mut m,
                9,
                rel.members.iter().map(|&(id, ..)| {
                    let d = zigzag(id - prev);
                    prev = id;
                    d
                }),
            );
            packed(
                &mut m,
                10,
                rel.members.iter().map(|&(_, t, _)| match t {
                    MemberType::Node => 0,
                    MemberType::Way => 1,
                    MemberType::Relation => 2,
                }),
            );
            bytes(&mut group, 4, &m);
        }
        file_blob(w, "OSMData", &s.block(&group))?;
    }
    Ok(())
}

/// The text with the characters that XML attributes need escaped.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' => o.push_str("&#9;"),
            c => o.push(c),
        }
    }
    o
}

/// Write the cut as an `.osm` XML file (OSM XML 0.6, no metadata). The coordinates have 7 decimals, as the PBF.
pub fn write_xml<W: Write>(raw: &Raw, w: &mut W) -> Result<(), String> {
    let e = |e: std::io::Error| e.to_string();
    let mut w = std::io::BufWriter::new(w);
    let tags = |w: &mut std::io::BufWriter<&mut W>, tags: &RawTags| -> Result<(), String> {
        for (k, v) in tags {
            writeln!(w, "    <tag k=\"{}\" v=\"{}\"/>", esc(k), esc(v)).map_err(e)?;
        }
        Ok(())
    };
    writeln!(w, "<?xml version='1.0' encoding='UTF-8'?>").map_err(e)?;
    writeln!(
        w,
        "<osm version=\"0.6\" generator=\"duckosm-rust {}\">",
        env!("CARGO_PKG_VERSION")
    )
    .map_err(e)?;
    for n in &raw.nodes {
        let open = format!(
            "  <node id=\"{}\" lat=\"{:.7}\" lon=\"{:.7}\"",
            n.id, n.lat, n.lon
        );
        if n.tags.is_empty() {
            writeln!(w, "{open}/>").map_err(e)?;
        } else {
            writeln!(w, "{open}>").map_err(e)?;
            tags(&mut w, &n.tags)?;
            writeln!(w, "  </node>").map_err(e)?;
        }
    }
    for way in &raw.ways {
        writeln!(w, "  <way id=\"{}\">", way.id).map_err(e)?;
        for r in &way.refs {
            writeln!(w, "    <nd ref=\"{r}\"/>").map_err(e)?;
        }
        tags(&mut w, &way.tags)?;
        writeln!(w, "  </way>").map_err(e)?;
    }
    for rel in &raw.relations {
        writeln!(w, "  <relation id=\"{}\">", rel.id).map_err(e)?;
        for (id, t, role) in &rel.members {
            let t = match t {
                MemberType::Node => "node",
                MemberType::Way => "way",
                MemberType::Relation => "relation",
            };
            writeln!(
                w,
                "    <member type=\"{t}\" ref=\"{id}\" role=\"{}\"/>",
                esc(role)
            )
            .map_err(e)?;
        }
        tags(&mut w, &rel.tags)?;
        writeln!(w, "  </relation>").map_err(e)?;
    }
    writeln!(w, "</osm>").map_err(e)?;
    w.flush().map_err(e)
}
