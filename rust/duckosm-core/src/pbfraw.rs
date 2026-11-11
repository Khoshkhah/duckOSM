//! A fast decoder of the data blocks of a `.osm.pbf` file for duckOSM's `raw.*` (docs/design/streaming_load.md). It
//! reads the protobuf bytes in place, as `ST_READOSM` does: the string table is checked as UTF-8 one time for each
//! block, the keys, the values and the refs go into buffers that are used again, the metadata (`Info`) is skipped, and
//! the groups of the kinds that are not wanted are skipped. `osmpbf` builds owned objects with vectors for each way,
//! and it was the most of the time of the load (France, the way blocks: 6.2 s of the decode, 0.7 s of it is zlib).
//!
//! The format (https://wiki.openstreetmap.org/wiki/PBF_Format): a block is a 4-byte length, a `BlobHeader` and a
//! `Blob` (raw or zlib data). The data is a `PrimitiveBlock`: a string table and groups of nodes, dense nodes, ways
//! and relations. The coordinates are `1e-9 * (offset + granularity * value)`, as `osmpbf`.

use crate::graph::MemberType;
use crate::raw::RawSink;
use std::io::Read;

type R<T> = Result<T, String>;

fn bad(what: &str) -> String {
    format!("the .osm.pbf block is not valid: {what}")
}

/// A protobuf message, read in place.
#[derive(Clone, Copy)]
struct Msg<'a>(&'a [u8]);

impl<'a> Msg<'a> {
    fn varint(&mut self) -> R<u64> {
        let mut v = 0u64;
        for (i, &c) in self.0.iter().enumerate().take(10) {
            v |= ((c & 0x7f) as u64) << (7 * i);
            if c < 0x80 {
                self.0 = &self.0[i + 1..];
                return Ok(v);
            }
        }
        Err(bad("a varint"))
    }

    /// The next field: its number and its wire type, or None at the end.
    fn key(&mut self) -> R<Option<(u64, u64)>> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let k = self.varint()?;
        Ok(Some((k >> 3, k & 7)))
    }

    fn bytes(&mut self) -> R<&'a [u8]> {
        let n = self.varint()? as usize;
        let b = self.0.get(..n).ok_or_else(|| bad("a length"))?;
        self.0 = &self.0[n..];
        Ok(b)
    }

    fn skip(&mut self, wire: u64) -> R<()> {
        match wire {
            0 => self.varint().map(|_| ()),
            1 => self.take(8),
            2 => self.bytes().map(|_| ()),
            5 => self.take(4),
            _ => Err(bad("a wire type")),
        }
    }

    fn take(&mut self, n: usize) -> R<()> {
        self.0 = self.0.get(n..).ok_or_else(|| bad("a fixed field"))?;
        Ok(())
    }
}

fn zigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// The values of a packed varint field, into `out` (cleared first).
fn packed(b: &[u8], out: &mut Vec<u64>) -> R<()> {
    out.clear();
    let mut m = Msg(b);
    while !m.0.is_empty() {
        out.push(m.varint()?);
    }
    Ok(())
}

/// The values of a packed sint64 field with delta coding (ids, refs, coordinates), into `out`.
fn packed_delta(b: &[u8], out: &mut Vec<i64>) -> R<()> {
    out.clear();
    let (mut m, mut x) = (Msg(b), 0i64);
    while !m.0.is_empty() {
        x = x.wrapping_add(zigzag(m.varint()?));
        out.push(x);
    }
    Ok(())
}

/// Buffers that the decoder uses again for each element.
#[derive(Default)]
pub struct Buffers {
    data: Vec<u8>,
    a: Vec<u64>,
    b: Vec<u64>,
    c: Vec<u64>,
    ids: Vec<i64>,
    lats: Vec<i64>,
    lons: Vec<i64>,
}

/// The type and the data of a block (`block`: the 4-byte length, the `BlobHeader` and the `Blob`). The data of a
/// zlib block is inflated into `data`.
fn block_data<'a>(block: &'a [u8], data: &'a mut Vec<u8>) -> R<(bool, &'a [u8])> {
    let h = u32::from_be_bytes(block.get(..4).ok_or_else(|| bad("the length"))?.try_into().unwrap()) as usize;
    let header = block.get(4..4 + h).ok_or_else(|| bad("the header"))?;
    let mut m = Msg(header);
    let mut is_data = false;
    while let Some((f, w)) = m.key()? {
        if f == 1 && w == 2 {
            is_data = m.bytes()? == b"OSMData";
        } else {
            m.skip(w)?;
        }
    }
    let mut m = Msg(&block[4 + h..]);
    let (mut raw, mut zlib, mut size) = (None, None, 0usize);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 2) => raw = Some(m.bytes()?),
            (2, 0) => size = m.varint()? as usize,
            (3, 2) => zlib = Some(m.bytes()?),
            (4 | 6 | 7, 2) => return Err("this .osm.pbf block has a compression other than zlib (lzma, lz4 or zstd)".into()),
            _ => m.skip(w)?,
        }
    }
    if let Some(r) = raw {
        return Ok((is_data, r));
    }
    let z = zlib.ok_or_else(|| bad("no data"))?;
    data.clear();
    data.reserve(size);
    flate2::read::ZlibDecoder::new(z)
        .read_to_end(data)
        .map_err(|e| format!("the .osm.pbf block does not inflate: {e}"))?;
    Ok((is_data, data.as_slice()))
}

/// Decode one block of the file (`block`: as [`crate::pbf::BlockIndex`] gives it, the length, the header and the data)
/// and give the elements of the kinds in `keep` (nodes, ways, relations) to `sink`, in the order of the block.
pub fn decode_block(block: &[u8], keep: [bool; 3], buf: &mut Buffers, sink: &mut impl RawSink) -> R<()> {
    let mut data = std::mem::take(&mut buf.data);
    let r = (|| {
        let (is_data, d) = block_data(block, &mut data)?;
        if is_data {
            primitive_block(d, keep, buf, sink)?;
        }
        Ok(())
    })();
    buf.data = data;
    r
}

fn primitive_block(d: &[u8], keep: [bool; 3], buf: &mut Buffers, sink: &mut impl RawSink) -> R<()> {
    // the fields 17 to 20 come after the groups, so the block is read two times
    let (mut table, mut groups) = (None, Vec::new());
    let (mut gran, mut lat_off, mut lon_off) = (100i64, 0i64, 0i64);
    let mut m = Msg(d);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 2) => table = Some(m.bytes()?),
            (2, 2) => groups.push(m.bytes()?),
            (17, 0) => gran = m.varint()? as i64,
            (19, 0) => lat_off = m.varint()? as i64,
            (20, 0) => lon_off = m.varint()? as i64,
            _ => m.skip(w)?,
        }
    }
    let mut strings: Vec<&str> = Vec::new();
    let mut t = Msg(table.unwrap_or(&[]));
    while let Some((f, w)) = t.key()? {
        if f == 1 && w == 2 {
            strings.push(std::str::from_utf8(t.bytes()?).map_err(|_| bad("a string that is not UTF-8"))?);
        } else {
            t.skip(w)?;
        }
    }
    let s = strings.as_slice();
    let coord = |off: i64, v: i64| 1e-9 * (off + gran * v) as f64;
    for g in groups {
        let mut g = Msg(g);
        while let Some((f, w)) = g.key()? {
            match (f, w) {
                (1, 2) if keep[0] => node(g.bytes()?, buf, sink, s, &coord, lat_off, lon_off)?,
                (2, 2) if keep[0] => dense(g.bytes()?, buf, sink, s, &coord, lat_off, lon_off)?,
                (3, 2) if keep[1] => way(g.bytes()?, buf, sink, s)?,
                (4, 2) if keep[2] => relation(g.bytes()?, buf, sink, s)?,
                _ => g.skip(w)?,
            }
        }
    }
    Ok(())
}

/// Check that each string index is in the string table.
fn check(ix: &[u64], s: &[&str]) -> R<()> {
    match ix.iter().all(|&i| (i as usize) < s.len()) {
        true => Ok(()),
        false => Err(bad("a string index")),
    }
}

/// The tags from the string indexes in `keys` and `vals` (checked with [`check`]).
fn tags<'s, 'b>(keys: &'b [u64], vals: &'b [u64], s: &'b [&'s str]) -> impl Iterator<Item = (&'s str, &'s str)> + 'b {
    keys.iter().zip(vals).map(move |(&k, &v)| (s[k as usize], s[v as usize]))
}

fn node<'s>(
    b: &[u8],
    buf: &mut Buffers,
    sink: &mut impl RawSink,
    s: &[&'s str],
    coord: &impl Fn(i64, i64) -> f64,
    lat_off: i64,
    lon_off: i64,
) -> R<()> {
    let (mut id, mut lat, mut lon) = (0, 0, 0);
    buf.a.clear();
    buf.b.clear();
    let mut m = Msg(b);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 0) => id = zigzag(m.varint()?),
            (2, 2) => packed(m.bytes()?, &mut buf.a)?,
            (3, 2) => packed(m.bytes()?, &mut buf.b)?,
            (8, 0) => lat = zigzag(m.varint()?),
            (9, 0) => lon = zigzag(m.varint()?),
            _ => m.skip(w)?,
        }
    }
    check(&buf.a, s)?;
    check(&buf.b, s)?;
    sink.node(id, coord(lat_off, lat), coord(lon_off, lon), &mut tags(&buf.a, &buf.b, s));
    Ok(())
}

fn dense<'s>(
    b: &[u8],
    buf: &mut Buffers,
    sink: &mut impl RawSink,
    s: &[&'s str],
    coord: &impl Fn(i64, i64) -> f64,
    lat_off: i64,
    lon_off: i64,
) -> R<()> {
    buf.a.clear();
    let mut m = Msg(b);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 2) => packed_delta(m.bytes()?, &mut buf.ids)?,
            (8, 2) => packed_delta(m.bytes()?, &mut buf.lats)?,
            (9, 2) => packed_delta(m.bytes()?, &mut buf.lons)?,
            (10, 2) => packed(m.bytes()?, &mut buf.a)?,
            _ => m.skip(w)?,
        }
    }
    check(&buf.a, s)?;
    if buf.lats.len() != buf.ids.len() || buf.lons.len() != buf.ids.len() {
        return Err(bad("dense nodes with more ids than coordinates"));
    }
    // keys_vals: key, value, key, value, ..., 0 for each node; empty when no node of the block has tags
    let mut kv = 0;
    for i in 0..buf.ids.len() {
        let start = kv;
        while kv < buf.a.len() && buf.a[kv] != 0 {
            kv += 2;
        }
        let pairs = buf.a.get(start..kv).unwrap_or(&[]);
        kv += 1;
        let mut t = pairs.chunks_exact(2).map(|p| (s[p[0] as usize], s[p[1] as usize]));
        sink.node(buf.ids[i], coord(lat_off, buf.lats[i]), coord(lon_off, buf.lons[i]), &mut t);
    }
    Ok(())
}

fn way<'s>(b: &[u8], buf: &mut Buffers, sink: &mut impl RawSink, s: &[&'s str]) -> R<()> {
    let mut id = 0;
    buf.a.clear();
    buf.b.clear();
    buf.ids.clear();
    let mut m = Msg(b);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 0) => id = m.varint()? as i64,
            (2, 2) => packed(m.bytes()?, &mut buf.a)?,
            (3, 2) => packed(m.bytes()?, &mut buf.b)?,
            (8, 2) => packed_delta(m.bytes()?, &mut buf.ids)?,
            _ => m.skip(w)?,
        }
    }
    check(&buf.a, s)?;
    check(&buf.b, s)?;
    sink.way(id, &mut tags(&buf.a, &buf.b, s), &mut buf.ids.iter().copied());
    Ok(())
}

fn relation<'s>(b: &[u8], buf: &mut Buffers, sink: &mut impl RawSink, s: &[&'s str]) -> R<()> {
    let mut id = 0;
    buf.a.clear();
    buf.b.clear();
    buf.c.clear();
    buf.ids.clear();
    let mut roles = Vec::new();
    let mut m = Msg(b);
    while let Some((f, w)) = m.key()? {
        match (f, w) {
            (1, 0) => id = m.varint()? as i64,
            (2, 2) => packed(m.bytes()?, &mut buf.a)?,
            (3, 2) => packed(m.bytes()?, &mut buf.b)?,
            (8, 2) => packed(m.bytes()?, &mut roles)?,
            (9, 2) => packed_delta(m.bytes()?, &mut buf.ids)?,
            (10, 2) => packed(m.bytes()?, &mut buf.c)?,
            _ => m.skip(w)?,
        }
    }
    check(&buf.a, s)?;
    check(&buf.b, s)?;
    check(&roles, s)?;
    let mut members = Vec::with_capacity(buf.ids.len());
    for (i, &mid) in buf.ids.iter().enumerate() {
        let ty = match buf.c.get(i).copied().unwrap_or(0) {
            0 => MemberType::Node,
            1 => MemberType::Way,
            _ => MemberType::Relation,
        };
        let role = roles.get(i).map_or("", |&r| s[r as usize]);
        members.push((mid, ty, role));
    }
    sink.relation(id, &mut tags(&buf.a, &buf.b, s), &mut members.into_iter());
    Ok(())
}

/// Read the next block of a `.osm.pbf` file from `r`: its bytes (the length, the header and the data), or None at the
/// end of the file.
pub fn read_block<Rd: Read>(r: &mut Rd) -> R<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let h = u32::from_be_bytes(len) as usize;
    let mut block = len.to_vec();
    block.resize(4 + h, 0);
    r.read_exact(&mut block[4..]).map_err(|e| e.to_string())?;
    let mut m = Msg(&block[4..]);
    let mut size = None;
    while let Some((f, w)) = m.key()? {
        if f == 3 && w == 0 {
            size = Some(m.varint()? as usize);
        } else {
            m.skip(w)?;
        }
    }
    let size = size.ok_or_else(|| bad("a header with no data size"))?;
    let at = block.len();
    block.resize(at + size, 0);
    r.read_exact(&mut block[at..]).map_err(|e| e.to_string())?;
    Ok(Some(block))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The elements of a file, as a sink sees them.
    #[derive(Default, PartialEq, Debug)]
    struct All {
        nodes: Vec<(i64, u64, u64, Vec<(String, String)>)>,
        ways: Vec<(i64, Vec<(String, String)>, Vec<i64>)>,
        rels: Vec<(i64, Vec<(String, String)>, Vec<(i64, u8, String)>)>,
    }
    impl RawSink for All {
        fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
            let t = t.map(|(k, v)| (k.into(), v.into())).collect();
            self.nodes.push((id, lat.to_bits(), lon.to_bits(), t));
        }
        fn way<'a>(&mut self, id: i64, t: &mut dyn Iterator<Item = (&'a str, &'a str)>, r: &mut dyn Iterator<Item = i64>) {
            self.ways.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), r.collect()));
        }
        fn relation<'a>(
            &mut self,
            id: i64,
            t: &mut dyn Iterator<Item = (&'a str, &'a str)>,
            m: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
        ) {
            let m = m.map(|(i, ty, r)| (i, ty as u8, r.to_string())).collect();
            self.rels.push((id, t.map(|(k, v)| (k.into(), v.into())).collect(), m));
        }
    }

    /// The fast decoder gives the same elements, tags and coordinates (to the bit) as osmpbf, on Monaco.
    #[test]
    fn equals_osmpbf() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/monaco.osm.pbf");
        let mut want = All::default();
        let mut r = crate::pbf::BlobReader::from_path(&path).unwrap();
        for b in r.by_ref() {
            crate::pbf::raw_block(&b.unwrap(), &mut want).unwrap();
        }
        let mut got = All::default();
        let mut f = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        let mut buf = Buffers::default();
        while let Some(b) = read_block(&mut f).unwrap() {
            decode_block(&b, [true; 3], &mut buf, &mut got).unwrap();
        }
        assert!(!want.nodes.is_empty() && !want.ways.is_empty() && !want.rels.is_empty());
        assert_eq!(got, want);
    }
}
