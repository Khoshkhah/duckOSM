//! The raw cut as a file: `.osm.pbf` (write_pbf) and `.osm` XML (write_xml). The page gives the user the area
//! that it cut from a region file, as osmium's `smart` cut gives it (raw.rs), so that other tools can use it.
//!
//! The PBF has the header block and then blocks of 8000 nodes (dense), ways or relations, each in a zlib blob. The ids
//! are sorted in each type, so the header says `Sort.Type_then_ID`. A node has its coordinates in 1e-7 degrees (the
//! granularity of the standard); the files of Geofabrik have the same. There is no metadata (version, time, user).
//! The bytes do not depend on the target: `miniz_oxide` is pure Rust. The browser build and the native build write the
//! same file.

use crate::graph::{MemberType, Relation};
use crate::raw::{Raw, RawNode, RawTags, RawWay};
use crate::util::FxHashMap;
use std::io::Write;

/// Entities in one block of the PBF (the number that osmium uses).
pub(crate) const BLOCK: usize = 8000;

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

/// The string table of one block. Index 0 is the empty string, as the standard asks, and it is not used for a string
/// of the data: in the `keys_vals` of the dense nodes, 0 is the end of the tags of a node. An empty key or value (OSM
/// has tags such as `air_conditioning=`) gets its own index, as osmium writes it. Before, it got 0, and the tags of
/// the next nodes moved by one node for a reader (Île-de-France: about 2 300 nodes).
struct Strings<'a> {
    list: Vec<&'a str>,
    index: FxHashMap<&'a str, u64>,
}

impl<'a> Strings<'a> {
    fn new() -> Self {
        Strings {
            list: vec![""],
            index: FxHashMap::default(),
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

pub(crate) fn file_blob<W: Write>(w: &mut W, kind: &str, data: &[u8]) -> Result<(), String> {
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

/// Writes a `.osm.pbf` file from elements given one at a time, in the order of the file (all nodes, then all ways,
/// then all relations, each sorted by id): a block for each [`BLOCK`] elements of a kind. Only one block is in memory,
/// so a cut can write its file while it reads the region file ([`crate::cutstream`]).
pub struct PbfWriter<W: Write> {
    w: W,
    nodes: Vec<RawNode>,
    ways: Vec<RawWay>,
    relations: Vec<Relation>,
}

impl<W: Write> PbfWriter<W> {
    /// Write the header. `timestamp` (seconds since 1970) is the replication time of the region file's header.
    pub fn new(mut w: W, timestamp: Option<i64>) -> Result<Self, String> {
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
        file_blob(&mut w, "OSMHeader", &head)?;
        Ok(PbfWriter {
            w,
            nodes: Vec::with_capacity(BLOCK),
            ways: Vec::new(),
            relations: Vec::new(),
        })
    }

    pub fn node(&mut self, n: RawNode) -> Result<(), String> {
        self.nodes.push(n);
        if self.nodes.len() == BLOCK {
            self.flush_nodes()?;
        }
        Ok(())
    }

    pub fn way(&mut self, x: RawWay) -> Result<(), String> {
        self.flush_nodes()?;
        self.ways.push(x);
        if self.ways.len() == BLOCK {
            self.flush_ways()?;
        }
        Ok(())
    }

    pub fn relation(&mut self, r: Relation) -> Result<(), String> {
        self.flush_nodes()?;
        self.flush_ways()?;
        self.relations.push(r);
        if self.relations.len() == BLOCK {
            self.flush_relations()?;
        }
        Ok(())
    }

    /// Write blocks that are ready (each made by [`file_blob`]), after the elements given before.
    pub(crate) fn blocks(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.flush_nodes()?;
        self.flush_ways()?;
        self.flush_relations()?;
        self.w.write_all(bytes).map_err(|e| e.to_string())
    }

    /// Write the last blocks; gives the writer back.
    pub fn finish(mut self) -> Result<W, String> {
        self.flush_nodes()?;
        self.flush_ways()?;
        self.flush_relations()?;
        Ok(self.w)
    }

    fn flush_nodes(&mut self) -> Result<(), String> {
        if !self.nodes.is_empty() {
            file_blob(&mut self.w, "OSMData", &nodes_block(&self.nodes))?;
            self.nodes.clear();
        }
        Ok(())
    }

    fn flush_ways(&mut self) -> Result<(), String> {
        if !self.ways.is_empty() {
            file_blob(&mut self.w, "OSMData", &ways_block(&self.ways))?;
            self.ways.clear();
        }
        Ok(())
    }

    fn flush_relations(&mut self) -> Result<(), String> {
        if !self.relations.is_empty() {
            file_blob(&mut self.w, "OSMData", &relations_block(&self.relations))?;
            self.relations.clear();
        }
        Ok(())
    }
}

pub(crate) fn nodes_block(chunk: &[RawNode]) -> Vec<u8> {
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
    s.block(&group)
}

pub(crate) fn ways_block(chunk: &[RawWay]) -> Vec<u8> {
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
    s.block(&group)
}

pub(crate) fn relations_block(chunk: &[Relation]) -> Vec<u8> {
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
    s.block(&group)
}

/// Write the cut as a `.osm.pbf` file. `timestamp` (seconds since 1970) is the replication time of the region
/// file's header, when it has one.
pub fn write_pbf<W: Write>(raw: &Raw, timestamp: Option<i64>, w: &mut W) -> Result<(), String> {
    let mut out = PbfWriter::new(w, timestamp)?;
    for chunk in raw.nodes.chunks(BLOCK) {
        file_blob(&mut out.w, "OSMData", &nodes_block(chunk))?;
    }
    for chunk in raw.ways.chunks(BLOCK) {
        file_blob(&mut out.w, "OSMData", &ways_block(chunk))?;
    }
    for chunk in raw.relations.chunks(BLOCK) {
        file_blob(&mut out.w, "OSMData", &relations_block(chunk))?;
    }
    out.finish().map(|_| ())
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

#[cfg(all(test, feature = "pbf"))]
mod tests {
    use super::*;
    use crate::pbfraw::{decode_block, read_block, Buffers};
    use crate::raw::RawSink;

    #[derive(Default)]
    struct Nodes(Vec<(i64, RawTags)>);

    impl RawSink for Nodes {
        fn node<'a>(&mut self, id: i64, _: f64, _: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            self.0.push((id, tags.map(|(k, v)| (k.to_string(), v.to_string())).collect()));
        }
        fn way<'a>(&mut self, _: i64, _: &mut dyn Iterator<Item = (&'a str, &'a str)>, _: &mut dyn Iterator<Item = i64>) {}
        fn relation<'a>(
            &mut self,
            _: i64,
            _: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            _: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
        }
    }

    #[test]
    fn empty_tag_value_reads_back() {
        let t = |v: &[(&str, &str)]| v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<RawTags>();
        let node = |id, tags| RawNode { id, lon: 2.0, lat: 48.0, tags };
        let raw = Raw {
            nodes: vec![
                node(1, t(&[])),
                node(2, t(&[("air_conditioning", ""), ("name", "LEA")])),
                node(3, t(&[("amenity", "fast_food")])),
            ],
            ..Raw::default()
        };
        let mut file = Vec::new();
        write_pbf(&raw, None, &mut file).unwrap();
        let mut got = Nodes::default();
        let mut r = std::io::Cursor::new(file);
        while let Some(b) = read_block(&mut r).unwrap() {
            decode_block(&b, [true; 3], &mut Buffers::default(), &mut got).unwrap();
        }
        let want: Vec<(i64, RawTags)> = raw.nodes.iter().map(|n| (n.id, n.tags.clone())).collect();
        assert_eq!(got.0, want);
    }

    #[test]
    fn the_empty_string_of_the_data_is_not_index_0() {
        // index 0 ends the tags of a dense node: DuckDB's ST_READOSM (and so QuackOSM) moved the tags of the next nodes
        let mut s = Strings::new();
        assert_ne!(s.id(""), 0);
        assert_eq!(s.id("name"), 2);
    }
}
