//! "What's using C:" (DESIGN §3.12 item 2): one walk of a drive gives both views —
//! the **folder tree** (biggest first, drill down, Windows' own places locked) and the **file-type breakdown**.
//!
//! How it is fast without admin: a plain walk with `FindFirstFileExW` (basic info + large fetch, one call per folder
//! returns names *and* sizes), run on several threads at once. Junctions / symlinks are never followed. The NTFS MFT
//! read (seconds, WizTree-style) needs admin — not used here; see the order report.
//!
//! Memory is kept small: one 40-byte record per folder, names in one shared string, and per folder only its biggest
//! files by name (the rest is one "other files" sum). The whole result is dropped when the page closes.

use crate::classify::{classify_file, ClassRules, FileType, FolderClass};
use crate::{Result, StorageError, StorageOs, VolumeState};
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub type FolderId = u32;

/// Shared with the menu while a scan runs: progress to show, and Cancel.
#[derive(Debug, Default)]
pub struct ScanControl {
    cancel: AtomicBool,
    files: AtomicU64,
    folders: AtomicU64,
    bytes: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanProgress {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
}

impl ScanControl {
    pub fn new() -> Self {
        Self::default()
    }
    /// Stop the walk (the page closed). `scan_*` then returns `StorageError::Cancelled`.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn progress(&self) -> ScanProgress {
        ScanProgress {
            files: self.files.load(Ordering::Relaxed),
            folders: self.folders.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    /// Walker threads. Hard drives get fewer (seeking), SSDs more.
    pub threads: usize,
    /// Files kept by name per folder (the biggest); the rest become one "N other files" row.
    pub files_per_folder: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        ScanOptions { threads: cpus.clamp(2, 8), files_per_folder: 24 }
    }
}

impl ScanOptions {
    /// Threads for a drive: 2 on a hard drive (more just makes the head jump), the default otherwise.
    pub fn for_media(media: crate::MediaKind) -> Self {
        let mut o = Self::default();
        if media == crate::MediaKind::Hdd {
            o.threads = 2;
        }
        o
    }
}

const F_WINDOWS_OWN: u8 = 1;
const F_UNREADABLE: u8 = 2;
/// the folder's inside was cut away (`FolderTree::pruned`)
const F_PRUNED: u8 = 4;

#[derive(Debug, Clone, Copy)]
struct Node {
    parent: u32,
    name_off: u32,
    name_len: u32,
    first_child: u32,
    n_children: u32,
    files_off: u32,
    n_files: u32,
    other_files: u32,
    other_bytes: u64,
    total: u64,
    flags: u8,
}

#[derive(Debug, Clone, Copy)]
struct FileRec {
    name_off: u32,
    name_len: u32,
    bytes: u64,
}

/// The folder tree of one scan.
#[derive(Debug, Clone)]
pub struct FolderTree {
    root_path: PathBuf,
    nodes: Vec<Node>,
    names: String,
    files: Vec<FileRec>,
}

/// One row of the Folders view: a folder, a big file, or the "other files" sum.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderRow {
    pub name: String,
    pub bytes: u64,
    /// Share of the folder being looked at (0.0 – 1.0), for the share bar.
    pub share: f64,
    pub kind: RowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Folder {
        id: FolderId,
        /// Show "›" (it has folders inside).
        has_subfolders: bool,
        /// Windows' own place: lock, can't be opened.
        windows_own: bool,
        /// "Can't be read" (access denied).
        unreadable: bool,
    },
    File,
    /// "N other files" (files too small to list one by one).
    OtherFiles { count: u32 },
}

impl FolderTree {
    pub fn root(&self) -> FolderId {
        0
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    fn node(&self, id: FolderId) -> Result<&Node> {
        self.nodes.get(id as usize).ok_or_else(|| StorageError::NotFound(format!("folder #{id}")))
    }
    fn name_of(&self, off: u32, len: u32) -> &str {
        &self.names[off as usize..(off + len) as usize]
    }
    /// The folder's name ("C:" for a drive root).
    pub fn name(&self, id: FolderId) -> Result<String> {
        let n = self.node(id)?;
        Ok(if id == 0 { root_label(&self.root_path) } else { self.name_of(n.name_off, n.name_len).to_string() })
    }
    /// Bytes in the folder and everything under it.
    pub fn size(&self, id: FolderId) -> Result<u64> {
        Ok(self.node(id)?.total)
    }
    pub fn is_windows_own(&self, id: FolderId) -> Result<bool> {
        Ok(self.node(id)?.flags & F_WINDOWS_OWN != 0)
    }
    pub fn is_unreadable(&self, id: FolderId) -> Result<bool> {
        Ok(self.node(id)?.flags & F_UNREADABLE != 0)
    }
    /// Full path (for "Open in Explorer").
    pub fn path(&self, id: FolderId) -> Result<PathBuf> {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != 0 {
            let n = self.node(cur)?;
            parts.push(self.name_of(n.name_off, n.name_len));
            cur = n.parent;
        }
        let mut p = self.root_path.clone();
        for part in parts.iter().rev() {
            p.push(part);
        }
        Ok(p)
    }
    /// The path line "‹ C: › Users › Name   434 GB": (id, name, bytes) from the root down to `id`.
    pub fn breadcrumb(&self, id: FolderId) -> Result<Vec<(FolderId, String, u64)>> {
        let mut out = Vec::new();
        let mut cur = id;
        loop {
            let n = self.node(cur)?;
            out.push((cur, self.name(cur)?, n.total));
            if cur == 0 {
                break;
            }
            cur = n.parent;
        }
        out.reverse();
        Ok(out)
    }
    /// The parent folder ("‹" goes back up); `None` at the root.
    pub fn parent(&self, id: FolderId) -> Result<Option<FolderId>> {
        let n = self.node(id)?;
        Ok(if id == 0 { None } else { Some(n.parent) })
    }
    /// What is inside a folder, biggest first: its folders and its biggest files, then "N other files".
    /// Windows' own places refuse (`StorageError::WindowsOwn`) — the design shows them locked.
    pub fn rows(&self, id: FolderId) -> Result<Vec<FolderRow>> {
        let n = *self.node(id)?;
        if n.flags & F_WINDOWS_OWN != 0 {
            return Err(StorageError::WindowsOwn);
        }
        let share = |b: u64| if n.total == 0 { 0.0 } else { b as f64 / n.total as f64 };
        let mut rows = Vec::with_capacity((n.n_children + n.n_files + 1) as usize);
        for c in n.first_child..n.first_child + n.n_children {
            let child = &self.nodes[c as usize];
            rows.push(FolderRow {
                name: self.name_of(child.name_off, child.name_len).to_string(),
                bytes: child.total,
                share: share(child.total),
                kind: RowKind::Folder {
                    id: c,
                    has_subfolders: child.n_children > 0 || child.flags & F_PRUNED != 0,
                    windows_own: child.flags & F_WINDOWS_OWN != 0,
                    unreadable: child.flags & F_UNREADABLE != 0,
                },
            });
        }
        for f in &self.files[n.files_off as usize..(n.files_off + n.n_files) as usize] {
            rows.push(FolderRow {
                name: self.name_of(f.name_off, f.name_len).to_string(),
                bytes: f.bytes,
                share: share(f.bytes),
                kind: RowKind::File,
            });
        }
        rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        if n.other_files > 0 {
            rows.push(FolderRow {
                name: format!("{} other files", n.other_files),
                bytes: n.other_bytes,
                share: share(n.other_bytes),
                kind: RowKind::OtherFiles { count: n.other_files },
            });
        }
        Ok(rows)
    }
    /// The folder at `path` (case-insensitive), if the scan saw it.
    pub fn find(&self, path: &Path) -> Option<FolderId> {
        let rel = path.strip_prefix(&self.root_path).ok()?;
        let mut cur = 0u32;
        for part in rel.components() {
            let want = part.as_os_str().to_string_lossy().to_lowercase();
            let n = self.nodes[cur as usize];
            cur = (n.first_child..n.first_child + n.n_children).find(|&c| {
                let ch = &self.nodes[c as usize];
                self.name_of(ch.name_off, ch.name_len).to_lowercase() == want
            })?;
        }
        Some(cur)
    }
    /// What the tree holds on the heap (its node, file and name buffers) - what keeping it costs.
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.files.capacity() * std::mem::size_of::<FileRec>() + self.names.capacity()
    }

    /// The tree cut to what the page shows first (the owner Oct 8: no RAM unless you are doing something in it): the root,
    /// its own big files and its folders with their sizes - nothing below them. A folder that had folders inside is marked
    /// (`is_pruned`): opening it needs a scan of just that folder ([`graft`](Self::graft) puts it back in).
    pub fn pruned(&self) -> FolderTree {
        let mut t = FolderTree { root_path: self.root_path.clone(), nodes: Vec::new(), names: String::new(), files: Vec::new() };
        let r = self.nodes[0];
        let mut root = r;
        let push_name = |t: &mut FolderTree, off: u32, len: u32| -> (u32, u32) {
            let o = t.names.len() as u32;
            t.names.push_str(self.name_of(off, len));
            (o, len)
        };
        root.files_off = 0;
        root.first_child = 1;
        t.nodes.push(root);
        for f in &self.files[r.files_off as usize..(r.files_off + r.n_files) as usize] {
            let (o, l) = push_name(&mut t, f.name_off, f.name_len);
            t.files.push(FileRec { name_off: o, name_len: l, bytes: f.bytes });
        }
        for c in r.first_child..r.first_child + r.n_children {
            let mut n = self.nodes[c as usize];
            let (o, l) = push_name(&mut t, n.name_off, n.name_len);
            n.name_off = o;
            n.name_len = l;
            n.parent = 0;
            if n.n_children > 0 {
                n.flags |= F_PRUNED;
            }
            n.first_child = 0;
            n.n_children = 0;
            n.files_off = 0;
            n.n_files = 0;
            n.other_files = 0;
            n.other_bytes = 0;
            t.nodes.push(n);
        }
        t
    }

    /// The folder's inside was cut away by [`pruned`](Self::pruned) (open it = scan it again).
    pub fn is_pruned(&self, id: FolderId) -> bool {
        self.node(id).map(|n| n.flags & F_PRUNED != 0).unwrap_or(false)
    }

    /// Put a fresh scan of folder `at` (`sub` = [`scan_folder`] of `self.path(at)`) back into the tree: its folders, files
    /// and size replace the cut-away inside. The folders above keep their sizes.
    pub fn graft(&mut self, at: FolderId, sub: &FolderTree) -> Result<()> {
        self.node(at)?;
        let base = self.nodes.len() as u32;
        let (names0, files0) = (self.names.len() as u32, self.files.len() as u32);
        self.names.push_str(&sub.names);
        self.files.extend(sub.files.iter().map(|f| FileRec { name_off: f.name_off + names0, ..*f }));
        // sub node j (j >= 1) lands at base + j - 1; sub's root is `at`
        let map = |j: u32| if j == 0 { at } else { base + j - 1 };
        for n in &sub.nodes[1..] {
            self.nodes.push(Node {
                parent: map(n.parent),
                name_off: n.name_off + names0,
                first_child: if n.n_children > 0 { map(n.first_child) } else { 0 },
                files_off: n.files_off + files0,
                ..*n
            });
        }
        let s = sub.nodes[0];
        let a = &mut self.nodes[at as usize];
        a.first_child = if s.n_children > 0 { map(s.first_child) } else { 0 };
        a.n_children = s.n_children;
        a.files_off = s.files_off + files0;
        a.n_files = s.n_files;
        a.other_files = s.other_files;
        a.other_bytes = s.other_bytes;
        a.total = s.total;
        a.flags &= !F_PRUNED;
        Ok(())
    }}

fn root_label(root: &Path) -> String {
    let s = root.to_string_lossy();
    let s = s.trim_end_matches('\\');
    if s.len() == 2 && s.ends_with(':') {
        s.to_string()
    } else {
        root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.to_string())
    }
}

/// The file-type bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeBreakdown {
    /// Bytes the walk found per type (index = `FileType::index()`).
    pub walked: [u64; 6],
    /// Used bytes of the drive (total − free), when a whole drive was scanned.
    pub used_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeRow {
    pub ty: FileType,
    pub bytes: u64,
    /// Share of the used space (0.0 – 1.0).
    pub share: f64,
}

impl TypeBreakdown {
    pub fn walked_total(&self) -> u64 {
        self.walked.iter().sum()
    }
    /// Used space the walk did not see (unreadable folders, NTFS metadata) — negative when hard links (mostly in
    /// `C:\Windows\WinSxS`) were counted twice. `None` for a folder scan.
    pub fn unseen_bytes(&self) -> Option<i128> {
        self.used_bytes.map(|u| u as i128 - self.walked_total() as i128)
    }
    /// The bar's rows, biggest first, "Windows & other" last. For a whole drive, "Windows & other" is the drive's
    /// used space minus the five named types, so the parts add up exactly to "used" (it absorbs unreadable folders,
    /// NTFS metadata and hard links counted twice).
    pub fn rows(&self) -> Vec<TypeRow> {
        let named: u64 = FileType::ALL[..5].iter().map(|t| self.walked[t.index()]).sum();
        let other = match self.used_bytes {
            Some(used) => used.saturating_sub(named),
            None => self.walked[FileType::WindowsOther.index()],
        };
        let base = self.used_bytes.unwrap_or(named + other).max(1);
        let mut rows: Vec<TypeRow> = FileType::ALL[..5]
            .iter()
            .map(|&ty| TypeRow { ty, bytes: self.walked[ty.index()], share: self.walked[ty.index()] as f64 / base as f64 })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.bytes));
        rows.push(TypeRow { ty: FileType::WindowsOther, bytes: other, share: other as f64 / base as f64 });
        rows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanStats {
    pub files: u64,
    pub folders: u64,
    pub unreadable_folders: u64,
    /// Junctions / symlinks / mount points not followed.
    pub links_skipped: u64,
    pub elapsed: Duration,
    pub threads: usize,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub tree: FolderTree,
    pub types: TypeBreakdown,
    pub stats: ScanStats,
}

/// Scan one whole drive ("C"). A locked (BitLocker) drive returns `StorageError::Locked` — no scan.
pub fn scan_drive(os: &dyn StorageOs, letter: char, ctl: &ScanControl) -> Result<ScanResult> {
    scan_drive_with(os, letter, None, ctl)
}

/// [`scan_drive`] with the walker thread count chosen by the caller (`None` = by drive type).
pub fn scan_drive_with(os: &dyn StorageOs, letter: char, threads: Option<usize>, ctl: &ScanControl) -> Result<ScanResult> {
    let letter = letter.to_ascii_uppercase();
    let drive = os
        .drives()?
        .into_iter()
        .find(|d| d.letter == letter)
        .ok_or_else(|| StorageError::NotFound(format!("drive {letter}:")))?;
    match drive.state {
        VolumeState::Locked => return Err(StorageError::Locked(letter)),
        VolumeState::NotReady => return Err(StorageError::NotFound(format!("drive {letter}: is not ready"))),
        VolumeState::Ready => {}
    }
    let rules = ClassRules::from_os(os);
    let mut opts = ScanOptions::for_media(drive.media);
    if let Some(t) = threads {
        opts.threads = t.max(1);
    }
    let mut result = scan_folder(os, Path::new(&format!("{letter}:\\")), &rules, opts, ctl)?;
    result.types.used_bytes = Some(drive.total_bytes.saturating_sub(drive.free_bytes));
    result.types.free_bytes = Some(drive.free_bytes);
    Ok(result)
}

/// Windows' own places that the design locks (any drive): `X:\Windows`, `X:\$Recycle.Bin`,
/// `X:\System Volume Information`, `X:\Program Files\WindowsApps`.
pub fn is_windows_own(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let s = s.trim_end_matches('\\');
    let b = s.as_bytes();
    if b.len() < 4 || b[1] != b':' || b[2] != b'\\' || !b[0].is_ascii_alphabetic() {
        return false;
    }
    matches!(&s[3..], "windows" | "$recycle.bin" | "system volume information" | "program files\\windowsapps")
}

struct Job {
    id: u32,
    path: PathBuf,
    class: FolderClass,
    /// Still worth comparing paths against the rule folders (a parent of one of them).
    check_rules: bool,
}

struct Shared {
    nodes: Vec<Node>,
    names: String,
    files: Vec<FileRec>,
    queue: VecDeque<Job>,
    /// Jobs queued + being worked on.
    pending: usize,
    walked: [u64; 6],
    files_seen: u64,
    unreadable: u64,
    links: u64,
}

/// Scan one folder of a drive again (a folder whose inside was cut away by [`FolderTree::pruned`]), with the drive's
/// rules and speed settings - the result's tree goes back in with [`FolderTree::graft`].
pub fn scan_subfolder(os: &dyn StorageOs, letter: char, path: &Path, ctl: &ScanControl) -> Result<ScanResult> {
    let letter = letter.to_ascii_uppercase();
    let media = os.drives()?.into_iter().find(|d| d.letter == letter).map(|d| d.media).ok_or_else(|| StorageError::NotFound(format!("drive {letter}:")))?;
    let rules = ClassRules::from_os(os);
    scan_folder(os, path, &rules, ScanOptions::for_media(media), ctl)
}

/// Scan any folder (tests use this on a fake tree or a scratch folder).
pub fn scan_folder(
    os: &dyn StorageOs,
    root: &Path,
    rules: &ClassRules,
    opts: ScanOptions,
    ctl: &ScanControl,
) -> Result<ScanResult> {
    let start = Instant::now();
    let root_class = rules.folder_class(root);
    let mut st = Shared {
        nodes: vec![Node {
            parent: 0,
            name_off: 0,
            name_len: 0,
            first_child: 0,
            n_children: 0,
            files_off: 0,
            n_files: 0,
            other_files: 0,
            other_bytes: 0,
            total: 0,
            flags: if is_windows_own(root) { F_WINDOWS_OWN } else { 0 },
        }],
        names: String::new(),
        files: Vec::new(),
        queue: VecDeque::new(),
        pending: 1,
        walked: [0; 6],
        files_seen: 0,
        unreadable: 0,
        links: 0,
    };
    st.queue.push_back(Job {
        id: 0,
        path: root.to_path_buf(),
        class: root_class,
        check_rules: rules.may_contain_root(root),
    });
    // The root itself must be readable; anything below may fail ("Can't be read").
    if let Err(e) = os.read_dir(root) {
        return Err(match e.kind() {
            io::ErrorKind::NotFound => StorageError::NotFound(root.display().to_string()),
            io::ErrorKind::PermissionDenied => StorageError::NeedsAdmin(format!("reading {}", root.display())),
            _ => StorageError::Io(e),
        });
    }
    let state = Mutex::new(st);
    let wake = Condvar::new();
    let threads = opts.threads.max(1);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| worker(os, rules, opts, ctl, &state, &wake));
        }
    });
    if ctl.is_cancelled() {
        return Err(StorageError::Cancelled);
    }
    let st = state.into_inner().unwrap_or_else(|p| p.into_inner());
    let mut nodes = st.nodes;
    // Children always come after their parent, so one backwards pass adds every folder into its parent.
    for i in (1..nodes.len()).rev() {
        let t = nodes[i].total;
        let p = nodes[i].parent as usize;
        nodes[p].total += t;
    }
    let folders = nodes.len() as u64;
    Ok(ScanResult {
        tree: FolderTree { root_path: root.to_path_buf(), nodes, names: st.names, files: st.files },
        types: TypeBreakdown { walked: st.walked, used_bytes: None, free_bytes: None },
        stats: ScanStats {
            files: st.files_seen,
            folders,
            unreadable_folders: st.unreadable,
            links_skipped: st.links,
            elapsed: start.elapsed(),
            threads,
        },
    })
}

fn worker(
    os: &dyn StorageOs,
    rules: &ClassRules,
    opts: ScanOptions,
    ctl: &ScanControl,
    state: &Mutex<Shared>,
    wake: &Condvar,
) {
    loop {
        let job = {
            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if ctl.is_cancelled() {
                    st.queue.clear();
                    st.pending = 0;
                    wake.notify_all();
                    return;
                }
                if let Some(j) = st.queue.pop_front() {
                    break j;
                }
                if st.pending == 0 {
                    return;
                }
                st = wake.wait(st).unwrap_or_else(|p| p.into_inner());
            }
        };
        let listing = os.read_dir(&job.path);
        // Work out everything outside the lock.
        let mut subdirs: Vec<(String, FolderClass, bool, u8)> = Vec::new();
        let mut files: Vec<(String, u64)> = Vec::new();
        let mut own_bytes = 0u64;
        let mut walked = [0u64; 6];
        let mut links = 0u64;
        let mut n_files = 0u64;
        let unreadable = listing.is_err();
        for e in listing.unwrap_or_default() {
            if e.is_reparse {
                links += 1;
                continue;
            }
            if e.is_dir {
                let child = job.path.join(&e.name);
                // A rule folder wins over its parent's class (a Steam library inside Program Files is Games).
                let (class, check) = if job.check_rules {
                    let c = rules.folder_class(&child);
                    (if c == FolderClass::Plain { job.class } else { c }, rules.may_contain_root(&child))
                } else {
                    (job.class, false)
                };
                let flags = if is_windows_own(&child) { F_WINDOWS_OWN } else { 0 };
                subdirs.push((e.name, class, check, flags));
            } else {
                let size = if e.is_cloud_only { 0 } else { e.size };
                n_files += 1;
                own_bytes += size;
                walked[classify_file(job.class, &e.name).index()] += size;
                files.push((e.name, size));
            }
        }
        files.sort_by_key(|f| std::cmp::Reverse(f.1));
        let keep = files.len().min(opts.files_per_folder);
        let other_files = (files.len() - keep) as u32;
        let other_bytes: u64 = files[keep..].iter().map(|f| f.1).sum();
        files.truncate(keep);
        ctl.files.fetch_add(n_files, Ordering::Relaxed);
        ctl.folders.fetch_add(1, Ordering::Relaxed);
        ctl.bytes.fetch_add(own_bytes, Ordering::Relaxed);

        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
        let st = &mut *st;
        for (i, w) in walked.iter().enumerate() {
            st.walked[i] += w;
        }
        st.files_seen += n_files;
        st.links += links;
        if unreadable {
            st.unreadable += 1;
        }
        let files_off = st.files.len() as u32;
        for (name, bytes) in files {
            let name_off = st.names.len() as u32;
            st.names.push_str(&name);
            st.files.push(FileRec { name_off, name_len: name.len() as u32, bytes });
        }
        let first_child = st.nodes.len() as u32;
        let n_children = subdirs.len() as u32;
        for (name, class, check, flags) in subdirs {
            let name_off = st.names.len() as u32;
            st.names.push_str(&name);
            let id = st.nodes.len() as u32;
            st.nodes.push(Node {
                parent: job.id,
                name_off,
                name_len: name.len() as u32,
                first_child: 0,
                n_children: 0,
                files_off: 0,
                n_files: 0,
                other_files: 0,
                other_bytes: 0,
                total: 0,
                flags,
            });
            st.queue.push_back(Job { id, path: job.path.join(&name), class, check_rules: check });
            st.pending += 1;
        }
        let node = &mut st.nodes[job.id as usize];
        node.first_child = first_child;
        node.n_children = n_children;
        node.files_off = files_off;
        node.n_files = keep as u32;
        node.other_files = other_files;
        node.other_bytes = other_bytes;
        node.total = own_bytes;
        if unreadable {
            node.flags |= F_UNREADABLE;
        }
        st.pending = st.pending.saturating_sub(1);
        // Wake only as many sleepers as there is new work (all of them once everything is done).
        if st.pending == 0 {
            wake.notify_all();
        } else {
            for _ in 0..n_children.min(64) {
                wake.notify_one();
            }
        }
    }
}
