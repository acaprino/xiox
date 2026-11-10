//! Directory-size index built from the NTFS Master File Table.
//!
//! [`volume_index`] reads the MFT of one volume once (administrator rights needed) and folds
//! it into per-folder totals; [`DiskIndex`] implements `dc_model::DirIndex` on top of them.
//! File names are never kept, only folders have one: that keeps the peak memory of a whole
//! system volume low.

mod mft;

use dc_model::{DirIndex, IndexedDir, SizeInfo, normalize_path, path_key};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// "No node / no entry".
const NONE: u32 = u32::MAX;

/// Shared with the thread that reads the volumes.
#[derive(Debug, Default)]
pub struct Progress {
    /// MFT records read so far, all volumes together.
    pub records: AtomicU64,
    /// Set it to stop reading: the volume being read is abandoned.
    pub cancel: AtomicBool,
}

/// One MFT entry, as [`DiskIndex::from_nodes`] takes it.
///
/// `parent` is an index into the same `Vec<Node>`; `nodes[0]` is the volume root. `name` is
/// `Some` only for folders. `size`/`alloc` count for files only (a folder's own are ignored).
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub parent: u32,
    pub is_dir: bool,
    pub reparse: bool,
    pub name: Option<Box<str>>,
    pub size: u64,
    pub alloc: u64,
}

/// One folder of the index, with the totals of everything below it.
#[derive(Debug)]
struct Entry {
    /// Parent entry; `NONE` for a volume root.
    parent: u32,
    /// Own name as on disk; a volume root holds its path (`C:\`).
    name: Box<str>,
    logical: u64,
    alloc: u64,
    files: u64,
    reparse: bool,
}

/// Folder sizes of one or more NTFS volumes.
///
/// `by_key`: `path_key` of every folder -> entry. `by_name`: lowercase folder name -> entries.
/// The original-case path is rebuilt from the parent chain when asked for, instead of being
/// stored next to the key.
#[derive(Debug, Default)]
pub struct DiskIndex {
    entries: Vec<Entry>,
    by_key: HashMap<Box<str>, u32>,
    by_name: HashMap<Box<str>, Vec<u32>>,
}

impl DiskIndex {
    /// Build the index of one volume. Folders that cannot be reached from `nodes[0]`
    /// (unknown parent, parent that is not a folder, cycles, no name) are left out, and so
    /// are the files inside them.
    pub fn from_nodes(volume_root: &str, mut nodes: Vec<Node>) -> DiskIndex {
        const UNSEEN: u32 = u32::MAX;
        const BUSY: u32 = u32::MAX - 1;
        const ORPHAN: u32 = u32::MAX - 2;
        let n = nodes.len();
        let mut ix = DiskIndex::default();
        if n == 0 {
            return ix;
        }

        // Depth of every reachable folder (memoised climb, no per-file walk).
        let mut depth = vec![UNSEEN; n];
        depth[0] = 0;
        let mut chain: Vec<usize> = Vec::new();
        for i in 1..n {
            if !nodes[i].is_dir || depth[i] != UNSEEN {
                continue;
            }
            chain.clear();
            let mut cur = i;
            let found = loop {
                match depth[cur] {
                    UNSEEN => {
                        depth[cur] = BUSY;
                        chain.push(cur);
                        let p = nodes[cur].parent as usize;
                        if nodes[cur].name.is_none() || p >= n || (p != 0 && !nodes[p].is_dir) {
                            break ORPHAN;
                        }
                        cur = p;
                    }
                    BUSY => break ORPHAN,
                    d => break d,
                }
            };
            let mut d = found;
            for &c in chain.iter().rev() {
                if d != ORPHAN {
                    d += 1;
                }
                depth[c] = d;
            }
        }

        // Entries for reachable folders, in node order; `depth` becomes node -> entry.
        let mut root = normalize_path(Path::new(volume_root)).to_string_lossy().into_owned();
        if !root.ends_with('\\') {
            root.push('\\');
        }
        let mut entry_depth: Vec<u32> = Vec::new();
        let mut slot = depth;
        for i in 0..n {
            let d = slot[i];
            let is_folder = i == 0 || nodes[i].is_dir;
            if !is_folder || d >= ORPHAN {
                slot[i] = NONE;
                continue;
            }
            slot[i] = ix.entries.len() as u32;
            entry_depth.push(d);
            let node = &mut nodes[i];
            ix.entries.push(Entry {
                parent: NONE,
                name: if i == 0 { root.clone().into_boxed_str() } else { node.name.take().unwrap_or_default() },
                logical: 0,
                alloc: 0,
                files: 0,
                reparse: node.reparse,
            });
        }
        for i in 1..n {
            let s = slot[i];
            if s != NONE {
                ix.entries[s as usize].parent = slot[nodes[i].parent as usize];
            }
        }

        // Direct sums: every file to the folder that holds it.
        for node in nodes.iter().skip(1).filter(|f| !f.is_dir) {
            let p = node.parent as usize;
            if p < n && slot[p] != NONE {
                let e = &mut ix.entries[slot[p] as usize];
                e.logical += node.size;
                e.alloc += node.alloc;
                e.files += 1;
            }
        }
        drop(slot);
        drop(nodes);

        // Children into parents, deepest folders first.
        let mut order: Vec<u32> = (0..ix.entries.len() as u32).collect();
        order.sort_unstable_by_key(|&s| Reverse(entry_depth[s as usize]));
        drop(entry_depth);
        for &s in &order {
            let e = &ix.entries[s as usize];
            let (p, logical, alloc, files) = (e.parent, e.logical, e.alloc, e.files);
            if p != NONE {
                let pe = &mut ix.entries[p as usize];
                pe.logical += logical;
                pe.alloc += alloc;
                pe.files += files;
            }
        }

        // Keys, parents before children: path_key(parent) + "\" + lowercase name.
        let mut keys: Vec<String> = vec![String::new(); ix.entries.len()];
        ix.by_key.reserve(ix.entries.len());
        for &s in order.iter().rev() {
            let e = &ix.entries[s as usize];
            if e.parent == NONE {
                keys[s as usize] = path_key(Path::new(&*e.name));
                continue;
            }
            let lower = e.name.to_lowercase();
            let pk = &keys[e.parent as usize];
            let mut k = String::with_capacity(pk.len() + 1 + lower.len());
            k.push_str(pk);
            if !k.ends_with('\\') {
                k.push('\\');
            }
            k.push_str(&lower);
            keys[s as usize] = k;
            match ix.by_name.get_mut(lower.as_str()) {
                Some(v) => v.push(s),
                None => {
                    ix.by_name.insert(lower.into_boxed_str(), vec![s]);
                }
            }
        }
        drop(order);
        for (s, k) in keys.into_iter().enumerate() {
            ix.by_key.insert(k.into_boxed_str(), s as u32);
        }
        ix
    }

    /// Add the folders of another volume.
    pub fn merge(&mut self, other: DiskIndex) {
        let off = self.entries.len() as u32;
        self.entries.extend(other.entries.into_iter().map(|mut e| {
            if e.parent != NONE {
                e.parent += off;
            }
            e
        }));
        for (k, i) in other.by_key {
            self.by_key.insert(k, i + off);
        }
        for (k, v) in other.by_name {
            self.by_name.entry(k).or_default().extend(v.into_iter().map(|i| i + off));
        }
    }

    /// Number of folders in the index.
    pub fn dir_count(&self) -> usize {
        self.entries.len()
    }

    fn size_info(&self, i: u32) -> SizeInfo {
        let e = &self.entries[i as usize];
        SizeInfo { logical_bytes: e.logical, allocated_bytes: Some(e.alloc), file_count: e.files }
    }

    fn path_of(&self, i: u32) -> PathBuf {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = i;
        loop {
            let e = &self.entries[cur as usize];
            parts.push(&e.name);
            if e.parent == NONE {
                break;
            }
            cur = e.parent;
        }
        let mut s = String::new();
        for p in parts.iter().rev() {
            if !s.is_empty() && !s.ends_with('\\') {
                s.push('\\');
            }
            s.push_str(p);
        }
        PathBuf::from(s)
    }
}

impl DirIndex for DiskIndex {
    fn backend_name(&self) -> &'static str {
        "mft"
    }

    fn size_of(&self, dir: &Path) -> Option<SizeInfo> {
        self.by_key.get(path_key(dir).as_str()).map(|&i| self.size_info(i))
    }

    /// Folders whose name equals one of `names` (case-insensitive), at or below one of
    /// `roots`. Reparse-point folders (links, junctions) are skipped. Results in index order.
    fn find_dirs_named(&self, names: &[&str], roots: &[PathBuf]) -> Vec<IndexedDir> {
        let root_keys: Vec<String> = roots.iter().map(|r| path_key(r)).collect();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut hits: Vec<u32> = Vec::new();
        for n in names {
            if let Some(v) = self.by_name.get(n.to_lowercase().as_str()) {
                hits.extend(v.iter().copied().filter(|i| seen.insert(*i)));
            }
        }
        hits.sort_unstable();
        hits.into_iter()
            .filter(|&i| !self.entries[i as usize].reparse)
            .filter_map(|i| {
                let path = self.path_of(i);
                let key = path_key(&path);
                root_keys
                    .iter()
                    .any(|r| is_at_or_under(&key, r))
                    .then(|| IndexedDir { path, size: self.size_info(i) })
            })
            .collect()
    }
}

/// Both arguments are `path_key`s.
fn is_at_or_under(key: &str, root: &str) -> bool {
    if key == root {
        return true;
    }
    match key.strip_prefix(root) {
        Some(rest) => root.ends_with('\\') || rest.starts_with('\\'),
        None => false,
    }
}

/// Index of the whole volume `letter` (e.g. `'C'`), read from its MFT. Needs administrator
/// rights and an NTFS volume.
pub fn volume_index(letter: char, progress: &Progress) -> Result<DiskIndex, String> {
    let letter = letter.to_ascii_uppercase();
    if !letter.is_ascii_alphabetic() {
        return Err(format!("unità non valida: {letter}"));
    }
    let nodes = mft::read_volume(letter, progress)?;
    Ok(DiskIndex::from_nodes(&format!("{letter}:\\"), nodes))
}

fn drive_letter(p: &Path) -> Option<char> {
    let s = normalize_path(p).to_string_lossy().into_owned();
    let b = s.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| (b[0] as char).to_ascii_uppercase())
}

/// One index for the volumes holding `roots`, each volume read once.
///
/// `admin == false` -> `(None, [])`: the caller scans normally. A volume that cannot be read
/// (missing, not NTFS, access denied) gets a warning and is skipped; its roots are not in
/// the index (`size_of(root)` is `None`), so the caller has to scan them normally. Roots
/// without a drive letter (network paths) are skipped the same way.
pub fn build_for_roots(roots: &[PathBuf], admin: bool, progress: &Progress) -> (Option<DiskIndex>, Vec<String>) {
    let mut warnings = Vec::new();
    if !admin {
        return (None, warnings);
    }
    let mut letters: Vec<char> = Vec::new();
    for r in roots {
        match drive_letter(r) {
            Some(l) if !letters.contains(&l) => letters.push(l),
            Some(_) => {}
            None => warnings.push(format!(
                "{} non disponibile per la lettura veloce: uso la scansione normale.",
                normalize_path(r).display()
            )),
        }
    }
    letters.sort_unstable();
    let mut index: Option<DiskIndex> = None;
    for l in letters {
        match volume_index(l, progress) {
            Ok(ix) => match index.as_mut() {
                Some(all) => all.merge(ix),
                None => index = Some(ix),
            },
            Err(_) if progress.cancel.load(Ordering::Relaxed) => return (None, warnings),
            Err(_) => warnings.push(format!("Unità {l}: non disponibile per la lettura veloce: uso la scansione normale.")),
        }
    }
    (index, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::DirIndex;
    use std::path::{Path, PathBuf};

    fn dir(parent: u32, name: &str) -> Node {
        Node { parent, is_dir: true, reparse: false, name: Some(name.into()), size: 0, alloc: 0 }
    }
    fn file(parent: u32, size: u64, alloc: u64) -> Node {
        Node { parent, is_dir: false, reparse: false, name: None, size, alloc }
    }

    // 0 C:\  1 Projects  2 app  3 target  4 f(100/4096) in target  5 f(100/4096) in target
    // 6 f(10/0) in app  7 link (dir, reparse) in Projects named "target"
    fn nodes() -> Vec<Node> {
        vec![
            dir(0, ""),
            dir(0, "Projects"),
            dir(1, "app"),
            dir(2, "target"),
            file(3, 100, 4096),
            file(3, 100, 4096),
            file(2, 10, 0),
            Node { reparse: true, ..dir(1, "target") },
        ]
    }
    fn sample() -> DiskIndex {
        DiskIndex::from_nodes(r"C:\", nodes())
    }

    #[test]
    fn sizes_add_up_through_folders() {
        let ix = sample();
        let t = ix.size_of(Path::new(r"C:\Projects\app\target")).unwrap();
        assert_eq!((t.logical_bytes, t.allocated_bytes, t.file_count), (200, Some(8192), 2));
        let a = ix.size_of(Path::new(r"C:\Projects\app")).unwrap();
        assert_eq!((a.logical_bytes, a.allocated_bytes, a.file_count), (210, Some(8192), 3));
    }

    #[test]
    fn finds_dirs_by_name_under_roots_case_insensitive_without_links() {
        let hits = sample().find_dirs_named(&["TARGET"], &[PathBuf::from(r"C:\Projects")]);
        assert_eq!(
            hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
            vec![PathBuf::from(r"C:\Projects\app\target")]
        );
        assert!(sample().find_dirs_named(&["target"], &[PathBuf::from(r"C:\Other")]).is_empty());
    }

    #[test]
    fn merge_keeps_both_volumes() {
        let mut ix = sample();
        ix.merge(DiskIndex::from_nodes(r"D:\", nodes()));
        let c = ix.size_of(Path::new(r"C:\Projects\app")).unwrap();
        let d = ix.size_of(Path::new(r"d:\projects\APP\")).unwrap();
        assert_eq!((c.logical_bytes, d.logical_bytes), (210, 210));
        let hits = ix.find_dirs_named(&["target"], &[PathBuf::from(r"D:\")]);
        assert_eq!(
            hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
            vec![PathBuf::from(r"D:\Projects\app\target")]
        );
    }

    #[test]
    fn missing_drive_gives_warning_and_no_index() {
        let (ix, w) = build_for_roots(&[PathBuf::from(r"Q:\nowhere")], true, &Progress::default());
        assert!(ix.is_none());
        assert!(w.iter().any(|m| m.contains("Q:")));
    }

    #[test]
    fn not_admin_gives_no_index() {
        let (ix, w) = build_for_roots(&[PathBuf::from(r"C:\Windows")], false, &Progress::default());
        assert!(ix.is_none() && w.is_empty());
    }
}
