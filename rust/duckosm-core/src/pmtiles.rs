//! A PMTiles v3 writer (https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md), in pure Rust.
//!
//! The tiles come in the order of their tile ids and go to the file at once: the file holds the header and the root
//! directory in its first 16384 bytes, then the tile data, then the metadata and the leaf directories. In memory: the
//! directory entries (24 bytes for each tile) and the places of the small tiles, for the tiles that repeat (the sea, a
//! forest). The directories and the metadata have gzip compression.

use std::io::{self, Seek, SeekFrom, Write};

/// The start of the tile data: the header (127 bytes) and the root directory are before it.
const DATA_START: u64 = 16384;
const HEADER: usize = 127;
/// A tile of at most this many bytes can repeat: its hash finds the copy that the file has.
const SMALL: usize = 1024;

/// The tile id of tile (z, x, y): the tiles of the zooms before z, plus the place of (x, y) on the Hilbert curve of z.
pub fn tile_id(z: u8, mut x: u32, mut y: u32) -> u64 {
    let mut id = ((1u64 << (2 * z as u32)) - 1) / 3;
    let mut s = if z == 0 { 0 } else { 1u32 << (z - 1) };
    while s > 0 {
        let (rx, ry) = ((x & s) > 0, (y & s) > 0);
        id += (s as u64) * (s as u64) * ((3 * rx as u64) ^ ry as u64);
        if !ry {
            if rx {
                x = s.wrapping_sub(1).wrapping_sub(x);
                y = s.wrapping_sub(1).wrapping_sub(y);
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    id
}

/// The zoom, x and y of a tile id.
pub fn tile_zxy(id: u64) -> (u8, u32, u32) {
    let mut z = 0u8;
    let mut first = 0u64;
    while first + (1u64 << (2 * z as u32)) <= id {
        first += 1u64 << (2 * z as u32);
        z += 1;
    }
    let mut t = id - first;
    let (mut x, mut y) = (0u32, 0u32);
    let mut s = 1u32;
    while s < (1u32 << z) {
        let rx = (1 & (t / 2)) as u32;
        let ry = (1 & (t ^ rx as u64)) as u32;
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        x += s * rx;
        y += s * ry;
        t /= 4;
        s *= 2;
    }
    (z, x, y)
}

/// gzip of `data` (RFC 1952), with the deflate of miniz_oxide.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8))
}

#[derive(Clone, Copy)]
struct Entry {
    id: u64,
    offset: u64,
    length: u32,
    run: u32,
}

/// The header fields that the caller gives.
pub struct Info {
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// lon/lat box: west, south, east, north (degrees)
    pub bounds: [f64; 4],
    pub center_zoom: u8,
    /// The metadata JSON (with `vector_layers` for vector tiles).
    pub metadata: String,
}

/// Writes a PMTiles file of MVT tiles with gzip compression, one tile at a time in the order of the tile ids.
pub struct Writer<W: Write + Seek> {
    w: W,
    at: u64,
    entries: Vec<Entry>,
    small: crate::util::FxHashMap<u64, (u64, u32)>,
    addressed: u64,
    contents: u64,
    last: Option<u64>,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut w: W) -> io::Result<Self> {
        w.seek(SeekFrom::Start(0))?;
        w.write_all(&vec![0u8; DATA_START as usize])?;
        Ok(Writer { w, at: 0, entries: Vec::new(), small: Default::default(), addressed: 0, contents: 0, last: None })
    }

    /// Adds tile `id` (its gzip bytes); the ids must grow.
    pub fn add(&mut self, id: u64, tile: &[u8]) -> io::Result<()> {
        if self.last.is_some_and(|l| l >= id) {
            return Err(io::Error::other(format!("pmtiles: tile id {id} after {}", self.last.unwrap())));
        }
        self.last = Some(id);
        self.addressed += 1;
        let hash = (tile.len() <= SMALL).then(|| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            tile.hash(&mut h);
            h.finish()
        });
        let (offset, length) = match hash.and_then(|h| self.small.get(&h).copied()) {
            Some(place) => place,
            None => {
                let place = (self.at, tile.len() as u32);
                self.w.write_all(tile)?;
                self.at += tile.len() as u64;
                self.contents += 1;
                if let Some(h) = hash {
                    self.small.insert(h, place);
                }
                place
            }
        };
        // a run: the same tile for the next id
        if let Some(e) = self.entries.last_mut() {
            if e.offset == offset && e.length == length && e.id + e.run as u64 == id {
                e.run += 1;
                return Ok(());
            }
        }
        self.entries.push(Entry { id, offset, length, run: 1 });
        Ok(())
    }

    /// The tiles that were added.
    pub fn addressed(&self) -> u64 {
        self.addressed
    }

    /// Writes the metadata, the directories and the header. Gives the writer back.
    pub fn finish(mut self, info: &Info) -> io::Result<W> {
        let data_len = self.at;
        let metadata = gzip(info.metadata.as_bytes());
        let (root, leaves) = directories(&self.entries);
        if HEADER + root.len() > DATA_START as usize {
            return Err(io::Error::other("pmtiles: the root directory is too large"));
        }
        let meta_at = DATA_START + data_len;
        self.w.write_all(&metadata)?;
        let leaves_at = meta_at + metadata.len() as u64;
        self.w.write_all(&leaves)?;
        let mut h = Vec::with_capacity(HEADER);
        h.extend_from_slice(b"PMTiles");
        h.push(3);
        for v in [
            HEADER as u64,
            root.len() as u64,
            meta_at,
            metadata.len() as u64,
            leaves_at,
            leaves.len() as u64,
            DATA_START,
            data_len,
            self.addressed,
            self.entries.len() as u64,
            self.contents,
        ] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        // clustered, internal compression gzip, tile compression gzip, tile type MVT
        h.extend_from_slice(&[1, 2, 2, 1, info.min_zoom, info.max_zoom]);
        let e7 = |v: f64| ((v * 1e7).round() as i32).to_le_bytes();
        let [w, s, e, n] = info.bounds;
        for v in [w, s, e, n] {
            h.extend_from_slice(&e7(v));
        }
        h.push(info.center_zoom);
        h.extend_from_slice(&e7((w + e) / 2.0));
        h.extend_from_slice(&e7((s + n) / 2.0));
        debug_assert_eq!(h.len(), HEADER);
        self.w.seek(SeekFrom::Start(0))?;
        self.w.write_all(&h)?;
        self.w.write_all(&root)?;
        self.w.flush()?;
        Ok(self.w)
    }
}

/// A directory of `entries` (the varint columns of the spec), with gzip.
fn directory(entries: &[Entry]) -> Vec<u8> {
    let mut b = Vec::new();
    crate::mvt::varint(&mut b, entries.len() as u64);
    let mut last = 0;
    for e in entries {
        crate::mvt::varint(&mut b, e.id - last);
        last = e.id;
    }
    entries.iter().for_each(|e| crate::mvt::varint(&mut b, e.run as u64));
    entries.iter().for_each(|e| crate::mvt::varint(&mut b, e.length as u64));
    for (i, e) in entries.iter().enumerate() {
        let next = i > 0 && e.offset == entries[i - 1].offset + entries[i - 1].length as u64;
        crate::mvt::varint(&mut b, if next { 0 } else { e.offset + 1 });
    }
    gzip(&b)
}

/// The root directory and the leaf directories: one root when it fits the first 16384 bytes, else leaves of more and
/// more entries until the root of the leaves fits.
fn directories(entries: &[Entry]) -> (Vec<u8>, Vec<u8>) {
    let root = directory(entries);
    if HEADER + root.len() <= DATA_START as usize {
        return (root, Vec::new());
    }
    let mut size = 4096;
    loop {
        let mut leaves = Vec::new();
        let mut roots = Vec::new();
        for chunk in entries.chunks(size) {
            let leaf = directory(chunk);
            roots.push(Entry { id: chunk[0].id, offset: leaves.len() as u64, length: leaf.len() as u32, run: 0 });
            leaves.extend(leaf);
        }
        let root = directory(&roots);
        if HEADER + root.len() <= DATA_START as usize {
            return (root, leaves);
        }
        size *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_ids_of_the_spec() {
        // the tests of the reference implementation (go-pmtiles)
        assert_eq!(tile_id(0, 0, 0), 0);
        assert_eq!(tile_id(1, 0, 0), 1);
        assert_eq!(tile_id(1, 0, 1), 2);
        assert_eq!(tile_id(1, 1, 1), 3);
        assert_eq!(tile_id(1, 1, 0), 4);
        assert_eq!(tile_id(2, 0, 0), 5);
        assert_eq!(tile_id(12, 3423, 1763), 19078479);
        for (z, x, y) in [(0, 0, 0), (3, 5, 2), (12, 3423, 1763), (14, 8529, 5974), (14, 16383, 0)] {
            assert_eq!(tile_zxy(tile_id(z, x, y)), (z, x, y));
        }
    }

    #[test]
    fn gzip_reads_back() {
        let data = b"PMTiles PMTiles PMTiles";
        let g = gzip(data);
        let mut d = flate2::read::GzDecoder::new(&g[..]);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut d, &mut out).unwrap();
        assert_eq!(out, data);
    }
}
