//! Read 2 of the tile cut (docs/design/tile_cut_two_reads.md): each element of the region file goes straight into the
//! files of its tiles, once, in the order of the file. Read 1 (parts 1 to 4a of `cut`) gave the plan: the books of the
//! nodes and the ways, the ways in more tiles, the extra nodes of each tile (sorted rows), the extra ways and the tiles
//! of the relations. No part files, no part 5.

use super::*;

/// Each element of a block to its tiles: its own tiles (`own`) and its extra tiles (the rows of the block), the tiles
/// with a file only.
struct Route<'a> {
    own: &'a (dyn Fn(i64, &mut Vec<u32>) + Sync),
    extra: &'a [(i64, u32)],
    with_file: &'a [bool],
    raw: bool,
    tiles: Vec<u32>,
    out: Vec<(u32, Piece)>,
    all: All,
}

/// An element for a tile: the element, or (an element of many tiles) its block, packed one time for all of them.
enum Piece {
    Elem(Elem),
    Packed(std::sync::Arc<(Vec<u8>, CutCounts)>),
}

/// An element for more tiles than this is packed one time, and its block goes to each tile: a copy for each tile held
/// a boundary of a country hundreds of times (France: 2 GB at the relations).
const SHARED: usize = 8;

impl Route<'_> {
    /// The tiles of `id`; true when it has one.
    fn tiles_of(&mut self, id: i64) -> bool {
        self.tiles.clear();
        (self.own)(id, &mut self.tiles);
        self.tiles.extend(tiles_in(self.extra, id));
        self.tiles.retain(|&t| self.with_file[t as usize]);
        self.tiles.sort_unstable();
        self.tiles.dedup();
        !self.tiles.is_empty()
    }
    /// The element to each of its tiles: a copy for each tile but the last, which gets the element itself (most elements
    /// have one tile: no copy).
    fn take(&mut self) {
        for e in std::mem::take(&mut self.all.0) {
            if self.tiles.len() > SHARED {
                let mut w = PbfWriter::data(Vec::new()).raw_blocks(self.raw);
                let (mut n, mut last) = (CutCounts::default(), None);
                // the writer of a block takes any element: no error is possible here, but keep the element when it fails
                let packed = put(&mut w, &mut n, &mut last, e.clone()).and_then(|_| w.finish());
                if let Ok(bytes) = packed {
                    let shared = std::sync::Arc::new((bytes, n));
                    for &t in &self.tiles {
                        self.out.push((t, Piece::Packed(shared.clone())));
                    }
                    continue;
                }
            }
            let (last, others) = self.tiles.split_last().expect("a tile");
            for &t in others {
                self.out.push((t, Piece::Elem(e.clone())));
            }
            self.out.push((*last, Piece::Elem(e)));
        }
    }
}

impl RawSink for Route<'_> {
    fn node<'a>(&mut self, id: i64, lat: f64, lon: f64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>) {
        if self.tiles_of(id) {
            self.all.node(id, lat, lon, tags);
            self.take();
        }
    }
    fn way<'a>(&mut self, id: i64, tags: &mut dyn Iterator<Item = (&'a str, &'a str)>, refs: &mut dyn Iterator<Item = i64>) {
        if self.tiles_of(id) {
            self.all.way(id, tags, refs);
            self.take();
        }
    }
    fn relation<'a>(
        &mut self,
        id: i64,
        tags: &mut dyn Iterator<Item = (&'a str, &'a str)>,
        members: &mut dyn Iterator<Item = (i64, MemberType, &'a str)>,
    ) {
        if self.tiles_of(id) {
            self.all.relation(id, tags, members);
            self.take();
        }
    }
}

/// The tile files so far: the header of each file at its first bytes, and the numbers of elements of each tile. The bytes
/// go to `out` on this thread (in the browser the storage belongs to it).
pub(super) struct Writers<'o> {
    started: Vec<bool>,
    counts: Vec<CutCounts>,
    head: Vec<u8>,
    raw: bool,
    /// the bytes of a tile file to its place; true for the first bytes of the file (its header)
    out: Box<dyn FnMut(u32, &[u8], bool) -> Result<(), String> + 'o>,
}

impl<'o> Writers<'o> {
    pub(super) fn new(n: usize, timestamp: Option<i64>, raw: bool, out: Box<dyn FnMut(u32, &[u8], bool) -> Result<(), String> + 'o>) -> Result<Self, String> {
        let head = PbfWriter::new(Vec::new(), timestamp)?.finish()?;
        Ok(Writers { started: vec![false; n], counts: vec![CutCounts::default(); n], head, raw, out })
    }

    /// The numbers of elements of each tile; the writers go.
    pub(super) fn into_counts(self) -> Vec<CutCounts> {
        self.counts
    }

    /// The packed elements of one block, by tile, to the tile files.
    fn put_bytes(&mut self, made: Vec<(u32, Vec<u8>, CutCounts)>) -> Result<(), String> {
        for (t, bytes, n) in made {
            let i = t as usize;
            if !self.started[i] {
                self.started[i] = true;
                (self.out)(t, &self.head, true)?;
            }
            (self.out)(t, &bytes, false)?;
            let c = &mut self.counts[i];
            c.nodes += n.nodes;
            c.ways += n.ways;
            c.relations += n.relations;
        }
        Ok(())
    }
}

/// The blocks at `blocks` (ascending places) to the tiles: this thread reads each block and its extra rows (`extra`,
/// which gets the span of the block and gives its rows), the pool decodes and routes the blocks, and this thread gives
/// the elements to the writers, in the order of the file.
#[allow(clippy::too_many_arguments)]
pub(super) fn route_blocks<R: Read + Seek>(
    r: &mut R,
    blocks: &[(u64, Span)],
    kind: usize,
    mut extra: impl FnMut(Span) -> Result<Vec<(i64, u32)>, String>,
    own: &(dyn Fn(i64, &mut Vec<u32>) + Sync),
    with_file: &[bool],
    writers: &mut Writers,
) -> Result<(), String> {
    let mut kinds = [false; 3];
    kinds[kind] = true;
    let writers_raw = writers.raw;
    // one block for each thread in a batch (the pipeline holds three batches)
    let n = rayon::current_num_threads();
    let (mut i, mut pos) = (0, None);
    crate::cutstream::pipeline(
        || {
            let mut batch: Vec<(Vec<u8>, Vec<(i64, u32)>)> = Vec::with_capacity(n);
            while i < blocks.len() && batch.len() < n {
                let (at, span) = blocks[i];
                i += 1;
                let rows = extra(span)?;
                if pos != Some(at) {
                    r.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
                }
                let b = read_block(r)?.ok_or("a block after the end of the file")?;
                pos = Some(at + b.len() as u64);
                batch.push((b, rows));
            }
            Ok((!batch.is_empty()).then_some(batch))
        },
        |batch: &Vec<(Vec<u8>, Vec<(i64, u32)>)>| {
            batch
                .par_iter()
                .map_init(Buffers::default, |buf, (b, rows)| {
                    let mut s = Route { own, extra: rows, with_file, raw: writers_raw, tiles: Vec::new(), out: Vec::new(), all: All::default() };
                    decode_block(b, kinds, buf, &mut s)?;
                    // the elements of each tile packed at once (a stable sort keeps the order of the file in each tile):
                    // no element waits across blocks (a buffer for each tile took 5.4 GB and most of the time on France)
                    s.out.sort_by_key(|x| x.0);
                    let mut made: Vec<(u32, Vec<u8>, CutCounts)> = Vec::new();
                    let mut rest = s.out.into_iter().peekable();
                    while let Some(&(t, _)) = rest.peek() {
                        let mut w = PbfWriter::data(Vec::new()).raw_blocks(writers_raw);
                        let (mut n, mut last) = (CutCounts::default(), None);
                        while let Some((_, p)) = rest.next_if(|x| x.0 == t) {
                            match p {
                                Piece::Elem(e) => put(&mut w, &mut n, &mut last, e)?,
                                // a packed block after the elements before it (`blocks` writes them first)
                                Piece::Packed(shared) => {
                                    w.blocks(&shared.0)?;
                                    n.nodes += shared.1.nodes;
                                    n.ways += shared.1.ways;
                                    n.relations += shared.1.relations;
                                }
                            }
                        }
                        made.push((t, w.finish()?, n));
                    }
                    Ok(made)
                })
                .collect::<Result<Vec<_>, String>>()
        },
        |made, _| {
            for block in made {
                writers.put_bytes(block)?;
            }
            Ok(true)
        },
    )
}

/// Where the bytes of each tile file go: the file `z_x_y.osm.pbf` in `dir` (the build by tiles), the part "tile" in the
/// parts (a folder, or the store), or the sink (the browser).
pub(super) fn outputs<'o>(tiles: &'o [Tile], dir: Option<&'o Path>, parts: &'o Parts, sink: Option<&'o mut Sink<'_>>) -> Result<Box<dyn FnMut(u32, &[u8], bool) -> Result<(), String> + 'o>, String> {
    let e = |p: &Path, e: io::Error| format!("{}: {e}", p.display());
    if let Some(sink) = sink {
        return Ok(Box::new(move |t, b, _| sink(t, b)));
    }
    let append = move |path: PathBuf, b: &[u8], first: bool| -> Result<(), String> {
        let mut o = std::fs::OpenOptions::new();
        if first {
            o.write(true).create(true).truncate(true);
        } else {
            o.append(true);
        }
        o.open(&path).and_then(|mut f| f.write_all(b)).map_err(|x| e(&path, x))
    };
    Ok(match (dir, &parts.files) {
        (Some(dir), _) => Box::new(move |t, b, first| {
            let tile = &tiles[t as usize];
            append(dir.join(format!("{}_{}_{}.osm.pbf", tile.z, tile.x, tile.y)), b, first)
        }),
        (None, Files::Dir(d)) => Box::new(move |t, b, first| append(Parts::path(d, t, "tile"), b, first)),
        (None, Files::Store(c)) => Box::new(move |t, b, _| c.append((t, "tile"), b).map_err(|x| format!("a tile file of the tile cut: {x}"))),
    })
}
