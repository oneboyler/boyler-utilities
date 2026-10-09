//! The same [`ScanResult`] as the walk, built from a whole-drive LISTING (Order 069, the owner: Everything's index tells
//! what is on a drive in seconds instead of the walk's minute and a half). The listing is read in two parts - first every
//! folder, then every file - in pages, so the whole list is never in memory at once; what is kept is exactly what the walk
//! keeps (one record per folder, its biggest files by name, the rest as one "other files" sum, the file-type totals).

use super::*;
use crate::{DriveListing, ListEntry};
use std::collections::HashMap;

/// Entries asked for per page (a page of 100,000 is about 25 MB on the wire).
const PAGE: usize = 100_000;

/// A directory string as the maps key it: `/` -> `\`, no trailing `\`, lower case (`C:\Users\You` -> `c:\users\you`).
fn norm_dir(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    norm_dir_into(s, &mut out);
    out
}

fn norm_dir_into(s: &str, out: &mut String) {
    out.clear();
    for c in s.chars() {
        let c = if c == '/' { '\\' } else { c };
        out.extend(c.to_lowercase());
    }
    while out.ends_with('\\') {
        out.pop();
    }
}

struct Folder {
    /// the folder it is in (normalised)
    dir: String,
    name: String,
    /// its own path (normalised)
    full: String,
    /// how deep it is: its parent's backslashes + 1 (a drive's top folders are 1)
    depth: usize,
}

/// Build the walk's result from `listing`. `root` is the drive's top (`C:\`). A stop (`ctl`) between pages ends it with
/// `Cancelled`; any error the listing gives is returned (the caller then walks instead).
pub(super) fn scan_listing(listing: &mut dyn DriveListing, root: &Path, rules: &ClassRules, opts: ScanOptions, ctl: &ScanControl) -> Result<ScanResult> {
    let start = Instant::now();
    let root_key = norm_dir(&root.to_string_lossy());

    // ---- the folders
    let mut folders: Vec<Folder> = Vec::new();
    loop {
        if ctl.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        let page = listing.page(true, PAGE)?;
        if page.is_empty() {
            break;
        }
        for e in page {
            // the drive itself has no folder
            if e.dir.is_empty() {
                continue;
            }
            let dir = norm_dir(&e.dir);
            let full = format!("{dir}\\{}", norm_dir(&e.name));
            let depth = dir.matches('\\').count() + 1;
            folders.push(Folder { dir, name: e.name, full, depth });
        }
        ctl.folders.store(folders.len() as u64, Ordering::Relaxed);
    }
    // parents before children, and the children of one folder next to each other (strings with one start sit together)
    folders.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.full.cmp(&b.full)));

    let mut nodes: Vec<Node> = Vec::with_capacity(folders.len() + 1);
    let mut names = String::new();
    let mut ids: HashMap<String, u32> = HashMap::with_capacity(folders.len() + 1);
    let mut class: Vec<FolderClass> = Vec::with_capacity(folders.len() + 1);
    let new_node = |parent: u32, name_off: u32, name_len: u32, flags: u8| Node {
        parent,
        name_off,
        name_len,
        first_child: 0,
        n_children: 0,
        files_off: 0,
        n_files: 0,
        other_files: 0,
        other_bytes: 0,
        total: 0,
        flags,
    };
    nodes.push(new_node(0, 0, 0, if is_windows_own(root) { F_WINDOWS_OWN } else { 0 }));
    ids.insert(root_key, 0);
    class.push(rules.folder_class(root));
    let mut orphans = 0u64;
    for f in &folders {
        // a folder whose parent the listing does not have (cannot happen on a whole drive) is left out
        let Some(&pid) = ids.get(&f.dir) else {
            orphans += 1;
            continue;
        };
        // the same path twice (a page edge moved while the drive changed, or two names that differ only in how they are
        // lower-cased): the first one stays
        if ids.contains_key(&f.full) {
            orphans += 1;
            continue;
        }
        let id = nodes.len() as u32;
        let name_off = names.len() as u32;
        names.push_str(&f.name);
        let own_place = Path::new(&f.full);
        let flags = if is_windows_own(own_place) { F_WINDOWS_OWN } else { 0 };
        let c = rules.folder_class(own_place);
        class.push(if c == FolderClass::Plain { class[pid as usize] } else { c });
        nodes.push(new_node(pid, name_off, f.name.len() as u32, flags));
        // the siblings arrive one after the other: the first starts the range
        let p = &mut nodes[pid as usize];
        if p.n_children == 0 {
            p.first_child = id;
        }
        p.n_children += 1;
        ids.insert(f.full.clone(), id);
    }
    drop(folders);
    let n = nodes.len();

    // ---- the files
    let keep = opts.files_per_folder;
    let mut own = vec![0u64; n];
    let mut other_files = vec![0u32; n];
    let mut other_bytes = vec![0u64; n];
    let mut top: Vec<Vec<(u64, String)>> = vec![Vec::new(); n];
    let mut walked = [0u64; 6];
    let mut files_seen = 0u64;
    let mut buf = String::new();
    // when a folder's list grows past twice what is kept, the small ones are folded into its "other files" sum
    let fold = |v: &mut Vec<(u64, String)>, of: &mut u32, ob: &mut u64| {
        v.sort_by_key(|f| Reverse(f.0));
        for (size, _) in v.drain(keep.min(v.len())..) {
            *of += 1;
            *ob += size;
        }
    };
    loop {
        if ctl.is_cancelled() {
            return Err(StorageError::Cancelled);
        }
        let page = listing.page(false, PAGE)?;
        if page.is_empty() {
            break;
        }
        let (mut page_files, mut page_bytes) = (0u64, 0u64);
        for ListEntry { dir, name, size, cloud_only } in page {
            norm_dir_into(&dir, &mut buf);
            let Some(&id) = ids.get(buf.as_str()) else { continue };
            let id = id as usize;
            let size = if cloud_only { 0 } else { size };
            own[id] += size;
            page_files += 1;
            page_bytes += size;
            walked[classify_file(class[id], &name).index()] += size;
            let v = &mut top[id];
            v.push((size, name));
            if v.len() > keep.saturating_mul(2).max(8) {
                fold(v, &mut other_files[id], &mut other_bytes[id]);
            }
        }
        files_seen += page_files;
        ctl.files.fetch_add(page_files, Ordering::Relaxed);
        ctl.bytes.fetch_add(page_bytes, Ordering::Relaxed);
    }
    drop(ids);

    // ---- one record per folder, its biggest files by name
    let mut files: Vec<FileRec> = Vec::new();
    for (id, node) in nodes.iter_mut().enumerate() {
        let v = &mut top[id];
        fold(v, &mut other_files[id], &mut other_bytes[id]);
        node.files_off = files.len() as u32;
        node.n_files = v.len() as u32;
        node.other_files = other_files[id];
        node.other_bytes = other_bytes[id];
        node.total = own[id];
        for (bytes, name) in v.drain(..) {
            let name_off = names.len() as u32;
            names.push_str(&name);
            files.push(FileRec { name_off, name_len: name.len() as u32, bytes });
        }
    }
    // children always come after their parent: one backwards pass adds every folder into its parent
    for i in (1..nodes.len()).rev() {
        let t = nodes[i].total;
        let p = nodes[i].parent as usize;
        nodes[p].total += t;
    }
    let folders_n = nodes.len() as u64;
    let tree = FolderTree { root_path: root.to_path_buf(), nodes, names, files };
    let biggest = biggest_of(&tree);
    Ok(ScanResult {
        tree,
        biggest,
        types: TypeBreakdown { walked, used_bytes: None, free_bytes: None },
        stats: ScanStats { files: files_seen, folders: folders_n, unreadable_folders: orphans, links_skipped: 0, elapsed: start.elapsed(), threads: 1 },
    })
}
