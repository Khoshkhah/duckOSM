//! The named files of the build by tiles (docs/design/build_by_tiles.md): the tile files, the tables of each tile and the
//! tables of the union. Natively they are files of `std::fs`. A page that has no folders (the browser) sets its own files
//! for the calling thread ([`set`], for example files in OPFS), and the same code of the cut, the tile builds and the union
//! then runs there. The functions have the names of `std::fs`.
//!
//! With a page's files, only the thread that set them sees them: a part that opens a file on another thread (rayon) gets
//! `std::fs`. The browser runs these parts with one thread.

use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use crate::spill::Blob;

/// The files of a page. A path is only a name: the page keeps no folders ([`create_dir_all`] does nothing).
pub trait Fs {
    /// A file to read (and to seek in).
    fn open(&self, p: &Path) -> io::Result<Box<dyn Blob>>;
    /// A new empty file to write (an old file with the same name goes).
    fn create(&self, p: &Path) -> io::Result<Box<dyn Blob>>;
    /// The size of the file in bytes.
    fn len(&self, p: &Path) -> io::Result<u64>;
    fn remove_file(&self, p: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// The files whose path starts with the folder `dir`, also in its sub folders.
    fn files(&self, dir: &Path) -> Vec<PathBuf>;
}

thread_local! {
    static FS: RefCell<Option<Rc<dyn Fs>>> = const { RefCell::new(None) };
}

/// The files of the page for the calling thread; `None`: `std::fs` again.
pub fn set(fs: Option<Rc<dyn Fs>>) {
    FS.with(|f| *f.borrow_mut() = fs);
}

fn page() -> Option<Rc<dyn Fs>> {
    FS.with(|f| f.borrow().clone())
}

/// A file of [`File::open`] or [`File::create`].
pub struct File {
    inner: Inner,
    path: PathBuf,
}

enum Inner {
    Std(std::fs::File),
    // a lock: a reader of Parquet files shares the file between threads (`Sync`)
    Page(std::sync::Mutex<Box<dyn Blob>>),
}

impl File {
    pub fn open<P: AsRef<Path>>(p: P) -> io::Result<File> {
        let path = p.as_ref().to_path_buf();
        let inner = match page() {
            Some(fs) => Inner::Page(std::sync::Mutex::new(fs.open(&path)?)),
            None => Inner::Std(std::fs::File::open(&path)?),
        };
        Ok(File { inner, path })
    }

    pub fn create<P: AsRef<Path>>(p: P) -> io::Result<File> {
        let path = p.as_ref().to_path_buf();
        let inner = match page() {
            Some(fs) => Inner::Page(std::sync::Mutex::new(fs.create(&path)?)),
            None => Inner::Std(std::fs::File::create(&path)?),
        };
        Ok(File { inner, path })
    }

    /// A second reader of the same file, from its start (a file of the page: the file opened again).
    pub fn try_clone(&self) -> io::Result<File> {
        match &self.inner {
            Inner::Std(f) => Ok(File { inner: Inner::Std(f.try_clone()?), path: self.path.clone() }),
            Inner::Page(_) => File::open(&self.path),
        }
    }

    pub fn len(&self) -> io::Result<u64> {
        match &self.inner {
            Inner::Std(f) => Ok(f.metadata()?.len()),
            Inner::Page(b) => {
                let mut b = b.lock().expect("the file");
                let at = b.stream_position()?;
                let n = b.seek(SeekFrom::End(0))?;
                b.seek(SeekFrom::Start(at))?;
                Ok(n)
            }
        }
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Read for File {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            Inner::Std(f) => f.read(b),
            Inner::Page(f) => f.get_mut().expect("the file").read(b),
        }
    }
}

impl Write for File {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            Inner::Std(f) => f.write(b),
            Inner::Page(f) => f.get_mut().expect("the file").write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            Inner::Std(f) => f.flush(),
            Inner::Page(f) => f.get_mut().expect("the file").flush(),
        }
    }
}

impl Seek for File {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match &mut self.inner {
            Inner::Std(f) => f.seek(to),
            Inner::Page(f) => f.get_mut().expect("the file").seek(to),
        }
    }
}

pub fn read<P: AsRef<Path>>(p: P) -> io::Result<Vec<u8>> {
    let mut b = Vec::new();
    File::open(p)?.read_to_end(&mut b)?;
    Ok(b)
}

pub fn read_to_string<P: AsRef<Path>>(p: P) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(p: P, bytes: C) -> io::Result<()> {
    let mut f = File::create(p)?;
    f.write_all(bytes.as_ref())?;
    f.flush()
}

pub fn remove_file<P: AsRef<Path>>(p: P) -> io::Result<()> {
    match page() {
        Some(fs) => fs.remove_file(p.as_ref()),
        None => std::fs::remove_file(p),
    }
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<()> {
    match page() {
        Some(fs) => fs.rename(from.as_ref(), to.as_ref()),
        None => std::fs::rename(from, to),
    }
}

/// Whether the file (natively also a folder) is there.
pub fn exists<P: AsRef<Path>>(p: P) -> bool {
    match page() {
        Some(fs) => fs.len(p.as_ref()).is_ok(),
        None => p.as_ref().exists(),
    }
}

/// The size of a file in bytes.
pub fn len<P: AsRef<Path>>(p: P) -> io::Result<u64> {
    match page() {
        Some(fs) => fs.len(p.as_ref()),
        None => Ok(std::fs::metadata(p)?.len()),
    }
}

pub fn create_dir_all<P: AsRef<Path>>(p: P) -> io::Result<()> {
    match page() {
        Some(_) => Ok(()),
        None => std::fs::create_dir_all(p),
    }
}

/// Remove a folder and all its files.
pub fn remove_dir_all<P: AsRef<Path>>(p: P) -> io::Result<()> {
    match page() {
        Some(fs) => {
            for f in fs.files(p.as_ref()) {
                fs.remove_file(&f)?;
            }
            Ok(())
        }
        None => std::fs::remove_dir_all(p),
    }
}

/// A new folder for temporary files, `{name}-{n}` (natively in the temporary folder of the system, with the id of the
/// process; with the page's files under `tmp/`). Two calls give two folders.
pub fn temp_dir(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match page() {
        Some(_) => PathBuf::from("tmp").join(format!("{name}-{n}")),
        None => std::env::temp_dir().join(format!("{name}-{}-{n}", std::process::id())),
    }
}

/// The storage under a [`Blocks`] store: one file that reads and writes at an offset (in the browser, one open file of OPFS).
pub trait Backing {
    fn read_at(&self, b: &mut [u8], at: u64) -> io::Result<usize>;
    fn write_at(&self, b: &[u8], at: u64) -> io::Result<()>;
}

/// The size of a block of a [`Blocks`] store.
const BLOCK: u64 = 1 << 20;

/// Many files in one file of a [`Backing`]: each file is a list of blocks of 1 MiB. A freed block is used again, and the file
/// grows when no block is free. The page has a fixed number of open files (a file of OPFS opens only asynchronously, and the
/// steps are synchronous), so the temporary files of a step, whose number grows with the area (the parts of the bus routes,
/// the runs of the sorts), go here: [`Blocks::named`] for the named files of [`Fs`], and the store of `spill` (unnamed).
pub struct Blocks<B: Backing> {
    back: B,
    free: RefCell<Vec<u64>>,
    next: std::cell::Cell<u64>,
    named: RefCell<std::collections::HashMap<PathBuf, Rc<RefCell<Extent>>>>,
}

/// The blocks of one file and its length.
#[derive(Default)]
struct Extent {
    blocks: Vec<u64>,
    len: u64,
}

/// A file of a [`Blocks`] store. An unnamed file frees its blocks when it is dropped; a named file when it is removed.
pub struct BlockFile<B: Backing> {
    store: Rc<Blocks<B>>,
    e: Rc<RefCell<Extent>>,
    pos: u64,
    named: bool,
}

// SAFETY: a store is set for one thread (`set`, `spill::set_temp_store`): its files stay on that thread.
unsafe impl<B: Backing> Send for BlockFile<B> {}

impl<B: Backing> Blocks<B> {
    pub fn new(back: B) -> Rc<Blocks<B>> {
        Rc::new(Blocks { back, free: RefCell::new(Vec::new()), next: std::cell::Cell::new(0), named: RefCell::new(Default::default()) })
    }

    fn block(&self) -> u64 {
        self.free.borrow_mut().pop().unwrap_or_else(|| {
            let b = self.next.get();
            self.next.set(b + 1);
            b
        })
    }

    fn release(&self, e: &mut Extent) {
        self.free.borrow_mut().append(&mut e.blocks);
        e.len = 0;
    }

    /// A new unnamed file (a temporary file of `spill`).
    pub fn temp(self: &Rc<Self>) -> BlockFile<B> {
        BlockFile { store: self.clone(), e: Default::default(), pos: 0, named: false }
    }

    /// The named file `p`: a new empty one (`create`), or the one that is there.
    pub fn named(self: &Rc<Self>, p: &Path, create: bool) -> io::Result<BlockFile<B>> {
        let mut named = self.named.borrow_mut();
        let e = if create {
            let e = named.entry(p.to_path_buf()).or_default().clone();
            self.release(&mut e.borrow_mut());
            e
        } else {
            named.get(p).cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{}: no such file", p.display())))?
        };
        Ok(BlockFile { store: self.clone(), e, pos: 0, named: true })
    }

    pub fn len(&self, p: &Path) -> Option<u64> {
        self.named.borrow().get(p).map(|e| e.borrow().len)
    }

    pub fn remove(&self, p: &Path) -> bool {
        let e = self.named.borrow_mut().remove(p);
        e.map(|e| self.release(&mut e.borrow_mut())).is_some()
    }

    pub fn rename(&self, from: &Path, to: &Path) -> bool {
        let Some(e) = self.named.borrow_mut().remove(from) else { return false };
        if let Some(old) = self.named.borrow_mut().insert(to.to_path_buf(), e) {
            self.release(&mut old.borrow_mut());
        }
        true
    }

    pub fn files(&self, dir: &Path) -> Vec<PathBuf> {
        self.named.borrow().keys().filter(|k| k.starts_with(dir)).cloned().collect()
    }
}

impl<B: Backing> Drop for BlockFile<B> {
    fn drop(&mut self) {
        if !self.named {
            self.store.release(&mut self.e.borrow_mut());
        }
    }
}

impl<B: Backing> Read for BlockFile<B> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let e = self.e.borrow();
        if self.pos >= e.len || out.is_empty() {
            return Ok(0);
        }
        let (k, off) = ((self.pos / BLOCK) as usize, self.pos % BLOCK);
        let n = (out.len() as u64).min(BLOCK - off).min(e.len - self.pos) as usize;
        let got = self.store.back.read_at(&mut out[..n], e.blocks[k] * BLOCK + off)?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl<B: Backing> Write for BlockFile<B> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if b.is_empty() {
            return Ok(0);
        }
        let mut e = self.e.borrow_mut();
        let (k, off) = ((self.pos / BLOCK) as usize, self.pos % BLOCK);
        while e.blocks.len() <= k {
            let blk = self.store.block();
            e.blocks.push(blk);
        }
        let n = (b.len() as u64).min(BLOCK - off) as usize;
        self.store.back.write_at(&b[..n], e.blocks[k] * BLOCK + off)?;
        self.pos += n as u64;
        e.len = e.len.max(self.pos);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<B: Backing> Seek for BlockFile<B> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let len = self.e.borrow().len;
        self.pos = match to {
            SeekFrom::Start(x) => x,
            SeekFrom::End(x) => len.saturating_add_signed(x),
            SeekFrom::Current(x) => self.pos.saturating_add_signed(x),
        };
        Ok(self.pos)
    }
}

/// The unnamed files of a [`Blocks`] store as the temporary store of `spill`.
pub struct BlockStore<B: Backing + 'static>(pub Rc<Blocks<B>>);

impl<B: Backing + 'static> crate::spill::Store for BlockStore<B> {
    fn create(&self) -> io::Result<Box<dyn Blob>> {
        Ok(Box::new(self.0.temp()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Files in memory, as a page keeps them.
    #[derive(Default)]
    struct Mem(RefCell<HashMap<PathBuf, Vec<u8>>>);

    /// A file of [`Mem`]: its bytes go into the map when it is dropped.
    struct MemFile {
        c: io::Cursor<Vec<u8>>,
        path: PathBuf,
        fs: Rc<Mem>,
    }
    // SAFETY: the test uses the files on one thread
    unsafe impl Send for MemFile {}

    impl Read for MemFile {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.c.read(b)
        }
    }
    impl Write for MemFile {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.c.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.fs.0.borrow_mut().insert(self.path.clone(), self.c.get_ref().clone());
            Ok(())
        }
    }
    impl Seek for MemFile {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            self.c.seek(to)
        }
    }
    impl Drop for MemFile {
        fn drop(&mut self) {
            let _ = self.flush();
        }
    }

    struct MemFs(Rc<Mem>);

    impl Fs for MemFs {
        fn open(&self, p: &Path) -> io::Result<Box<dyn Blob>> {
            let b = self.0 .0.borrow().get(p).cloned().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            Ok(Box::new(MemFile { c: io::Cursor::new(b), path: p.into(), fs: self.0.clone() }))
        }
        fn create(&self, p: &Path) -> io::Result<Box<dyn Blob>> {
            Ok(Box::new(MemFile { c: io::Cursor::new(Vec::new()), path: p.into(), fs: self.0.clone() }))
        }
        fn len(&self, p: &Path) -> io::Result<u64> {
            self.0 .0.borrow().get(p).map(|b| b.len() as u64).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn remove_file(&self, p: &Path) -> io::Result<()> {
            self.0 .0.borrow_mut().remove(p).map(|_| ()).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let b = self.0 .0.borrow_mut().remove(from).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            self.0 .0.borrow_mut().insert(to.into(), b);
            Ok(())
        }
        fn files(&self, dir: &Path) -> Vec<PathBuf> {
            self.0 .0.borrow().keys().filter(|k| k.starts_with(dir)).cloned().collect()
        }
    }

    /// Bytes in memory as the backing of a block store.
    #[derive(Default)]
    struct MemBack(RefCell<Vec<u8>>);

    impl Backing for MemBack {
        fn read_at(&self, b: &mut [u8], at: u64) -> io::Result<usize> {
            let v = self.0.borrow();
            let n = b.len().min(v.len().saturating_sub(at as usize));
            b[..n].copy_from_slice(&v[at as usize..at as usize + n]);
            Ok(n)
        }
        fn write_at(&self, b: &[u8], at: u64) -> io::Result<()> {
            let mut v = self.0.borrow_mut();
            if v.len() < at as usize + b.len() {
                v.resize(at as usize + b.len(), 0);
            }
            v[at as usize..at as usize + b.len()].copy_from_slice(b);
            Ok(())
        }
    }

    /// Many more files than a pool of open files (the parts of the bus routes of France: 402), in one backing: each file reads
    /// back its own bytes, also across blocks, and a removed or dropped file gives its blocks back.
    #[test]
    fn many_files_in_one_backing() {
        let store = Blocks::new(MemBack::default());
        let bytes = |k: usize| -> Vec<u8> { (0..(k * 7919) % (3 << 20)).map(|i| (i * 31 + k) as u8).collect() };
        for k in 0..1000 {
            let mut f = store.named(&PathBuf::from(format!("tmp/{k}")), true).unwrap();
            f.write_all(&bytes(k)).unwrap();
            if k % 2 == 1 {
                // the odd files go at once: their blocks are used again
                assert!(store.remove(&PathBuf::from(format!("tmp/{}", k))));
            }
        }
        for k in (0..1000).step_by(2) {
            let mut f = store.named(&PathBuf::from(format!("tmp/{k}")), false).unwrap();
            let mut got = Vec::new();
            f.read_to_end(&mut got).unwrap();
            assert!(got == bytes(k), "file {k}");
            f.seek(SeekFrom::Start(3)).unwrap();
        }
        let used = store.next.get();
        let free = store.free.borrow().len();
        assert!(free > 0, "the removed files gave blocks back");
        {
            let mut t = store.temp();
            t.write_all(&vec![1; free << 20]).unwrap();
        }
        assert_eq!(store.next.get(), used, "a temporary file takes free blocks first");
        assert_eq!(store.files(Path::new("tmp")).len(), 500);
        for k in (0..1000).step_by(2) {
            store.remove(&PathBuf::from(format!("tmp/{k}")));
        }
        assert_eq!(store.free.borrow().len() as u64, store.next.get(), "all blocks are free");
    }

    /// The page's files replace `std::fs` on the thread that set them, and `std::fs` comes back after `set(None)`.
    #[test]
    fn the_files_of_a_page() {
        let mem = Rc::new(Mem::default());
        set(Some(Rc::new(MemFs(mem.clone()))));
        write("out/a.bin", b"abc").unwrap();
        create_dir_all("out/no/folder").unwrap();
        assert!(exists("out/a.bin") && !exists("out/b.bin"));
        assert_eq!(len("out/a.bin").unwrap(), 3);
        rename("out/a.bin", "out/sub/b.bin").unwrap();
        let mut f = File::open("out/sub/b.bin").unwrap();
        f.seek(SeekFrom::Start(1)).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!((s.as_str(), f.len().unwrap()), ("bc", 3));
        assert!(temp_dir("x").starts_with("tmp"));
        remove_dir_all("out").unwrap();
        assert!(mem.0.borrow().is_empty());
        set(None);
        assert!(!exists("out/sub/b.bin"));
        assert!(temp_dir("x").starts_with(std::env::temp_dir()));
    }
}
