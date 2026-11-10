//! Filesystem primitives: verbatim (long) paths, size measurement, robust removal.
//!
//! Nothing here follows symlinks or junctions: a link met inside a tree is removed as a
//! link (never recursed into), and a link is never counted towards the measured size.

use dc_model::{SizeInfo, normalize_path};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

/// Attempts per entry = 1 + retries, with the backoff below between attempts.
const BACKOFF_MS: [u64; 3] = [50, 150, 400];

/// `\\?\C:\...` (or `\\?\UNC\...`) form so that paths longer than MAX_PATH work.
/// On other platforms the normalized path is returned unchanged.
pub fn verbatim(p: &Path) -> PathBuf {
    let n = normalize_path(p);
    if !cfg!(windows) {
        return n;
    }
    let s = n.to_string_lossy();
    match s.strip_prefix(r"\\") {
        Some(unc) => PathBuf::from(format!(r"\\?\UNC\{unc}")),
        None => PathBuf::from(format!(r"\\?\{s}")),
    }
}

/// True only when the OS positively says the path does not exist (access errors = not gone).
pub fn is_gone(p: &Path) -> bool {
    matches!(fs::symlink_metadata(verbatim(p)), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

fn round_up(len: u64, cluster: u64) -> u64 {
    if len == 0 { 0 } else { len.div_ceil(cluster) * cluster }
}

// ───────────────────────────── platform glue ─────────────────────────────

#[cfg(windows)]
mod sys {
    use std::fs::{self, OpenOptions};
    use std::io;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileIdInfo,
        FileStandardInfo, GetFileInformationByHandleEx,
    };

    const DELETE_ACCESS: u32 = 0x0001_0000;

    pub struct FileInfo {
        /// Real on-disk allocation (cluster rounded, compression/sparse aware, 0 if MFT-resident).
        pub alloc: u64,
        pub nlinks: u32,
        /// (volume serial, 128-bit file id), only fetched when `nlinks > 1`.
        pub id: Option<(u64, [u8; 16])>,
    }

    pub fn file_info(p: &Path) -> Option<FileInfo> {
        let f = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(p)
            .ok()?;
        let h = f.as_raw_handle();
        // SAFETY: `h` is a live handle owned by `f`; the out-buffers are correctly sized
        // plain-old-data structs for the requested information classes.
        unsafe {
            let mut std_info: FILE_STANDARD_INFO = std::mem::zeroed();
            if GetFileInformationByHandleEx(
                h,
                FileStandardInfo,
                (&mut std_info as *mut FILE_STANDARD_INFO).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            ) == 0
            {
                return None;
            }
            let mut id = None;
            if std_info.NumberOfLinks > 1 {
                let mut idi: FILE_ID_INFO = std::mem::zeroed();
                if GetFileInformationByHandleEx(
                    h,
                    FileIdInfo,
                    (&mut idi as *mut FILE_ID_INFO).cast(),
                    size_of::<FILE_ID_INFO>() as u32,
                ) != 0
                {
                    id = Some((idi.VolumeSerialNumber, idi.FileId.Identifier));
                }
            }
            Some(FileInfo { alloc: std_info.AllocationSize.max(0) as u64, nlinks: std_info.NumberOfLinks, id })
        }
    }

    /// Is this (link) entry a directory-type reparse point (junction / dir symlink)?
    pub fn link_is_dir(p: &Path) -> bool {
        fs::symlink_metadata(p).map(|m| m.file_attributes() & 0x10 != 0).unwrap_or(false)
    }

    /// First file under `root` that cannot be opened for delete because of a sharing/lock
    /// violation (ERROR_SHARING_VIOLATION 32 / ERROR_LOCK_VIOLATION 33).
    pub fn probe_blocking_file(root: &Path) -> Option<(std::path::PathBuf, io::Error)> {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    stack.push(entry.path());
                    continue;
                }
                let r = OpenOptions::new()
                    .access_mode(DELETE_ACCESS)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(entry.path());
                if let Err(e) = r
                    && matches!(e.raw_os_error(), Some(32 | 33))
                {
                    return Some((entry.path(), e));
                }
            }
        }
        None
    }
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    use std::path::{Path, PathBuf};

    pub struct FileInfo {
        pub alloc: u64,
        pub nlinks: u32,
        pub id: Option<(u64, [u8; 16])>,
    }
    pub fn file_info(_p: &Path) -> Option<FileInfo> {
        None
    }
    pub fn link_is_dir(p: &Path) -> bool {
        std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
    }
    pub fn probe_blocking_file(_root: &Path) -> Option<(PathBuf, io::Error)> {
        None
    }
}

pub use sys::probe_blocking_file;

// ─────────────────────────────── measuring ───────────────────────────────

/// Walk `root` (never through links) and measure it.
///
/// `logical_bytes` = sum of file lengths. `allocated_bytes` = real on-disk allocation as
/// reported by the filesystem; a hardlinked file counts only if ALL its links are inside
/// the tree (otherwise deleting the tree frees nothing for it). Unreadable entries are
/// skipped (so the figure is a lower bound, never an over-estimate).
pub fn measure(root: &Path) -> SizeInfo {
    let (mut logical, mut alloc, mut count) = (0u64, 0u64, 0u64);
    // id -> (alloc, links seen inside the tree, total links)
    let mut linked: HashMap<(u64, [u8; 16]), (u64, u32, u32)> = HashMap::new();
    let mut stack = vec![verbatim(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let path = entry.path();
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            logical += len;
            count += 1;
            match sys::file_info(&path) {
                Some(fi) if fi.nlinks > 1 => {
                    if let Some(id) = fi.id {
                        linked.entry(id).or_insert((fi.alloc, 0, fi.nlinks)).1 += 1;
                    }
                }
                Some(fi) => alloc += fi.alloc,
                None => alloc += round_up(len, 4096),
            }
        }
    }
    alloc += linked.values().filter(|(_, seen, n)| seen >= n).map(|(a, _, _)| a).sum::<u64>();
    SizeInfo { logical_bytes: logical, allocated_bytes: Some(alloc), file_count: count }
}

// ─────────────────────────────── removal ───────────────────────────────

#[derive(Default)]
pub struct RemoveReport {
    pub failures: u64,
    /// First (path, error) that blocked removal, in traversal order.
    pub first: Option<(PathBuf, io::Error)>,
}

impl RemoveReport {
    fn fail(&mut self, p: &Path, e: io::Error) {
        self.failures += 1;
        if self.first.is_none() {
            self.first = Some((normalize_path(p), e));
        }
    }
}

/// Clear the read-only attribute; true if it was set (so an immediate retry makes sense).
#[allow(clippy::permissions_set_readonly_false)] // Windows attribute bit, not a unix mode
fn clear_readonly(p: &Path) -> bool {
    match fs::symlink_metadata(p) {
        Ok(m) if !m.file_type().is_symlink() && m.permissions().readonly() => {
            let mut perm = m.permissions();
            perm.set_readonly(false);
            fs::set_permissions(p, perm).is_ok()
        }
        _ => false,
    }
}

/// Run `f`; on failure clear the read-only bit and retry immediately, then back off.
/// `patient = false` (a failure was already seen in this tree) skips the sleeping retries
/// so a tree full of locked files does not take minutes to report.
fn with_retry(p: &Path, patient: bool, mut f: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut slept = 0;
    let mut cleared = 0;
    loop {
        match f() {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                if cleared < 2 && clear_readonly(p) {
                    cleared += 1;
                    continue;
                }
                if !patient || slept >= BACKOFF_MS.len() {
                    return Err(e);
                }
                sleep(Duration::from_millis(BACKOFF_MS[slept]));
                slept += 1;
            }
        }
    }
}

fn remove_link(p: &Path, rep: &mut RemoveReport) {
    let patient = rep.failures == 0;
    let r = with_retry(p, patient, || if sys::link_is_dir(p) { fs::remove_dir(p) } else { fs::remove_file(p) });
    if let Err(e) = r {
        rep.fail(p, e);
    }
}

fn remove_dir_rec(dir: &Path, rep: &mut RemoveReport) {
    let before = rep.failures;
    match fs::read_dir(dir) {
        Err(e) => {
            rep.fail(dir, e);
            return;
        }
        Ok(rd) => {
            // Collect first: do not mutate the directory while iterating it.
            let entries: Vec<_> = rd.collect();
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        rep.fail(dir, e);
                        continue;
                    }
                };
                let p = entry.path();
                match entry.file_type() {
                    Err(e) => rep.fail(&p, e),
                    Ok(ft) if ft.is_symlink() => remove_link(&p, rep),
                    Ok(ft) if ft.is_dir() => remove_dir_rec(&p, rep),
                    Ok(_) => {
                        let patient = rep.failures == 0;
                        if let Err(e) = with_retry(&p, patient, || fs::remove_file(&p)) {
                            rep.fail(&p, e);
                        }
                    }
                }
            }
        }
    }
    // Only try the directory itself if everything below it is gone.
    if rep.failures == before {
        let patient = rep.failures == 0;
        if let Err(e) = with_retry(dir, patient, || fs::remove_dir(dir)) {
            rep.fail(dir, e);
        }
    }
}

/// Permanently remove `path`. If `path` itself is a link only the link is removed.
/// Links met inside the tree are removed as links, never followed.
pub fn remove_path(path: &Path) -> RemoveReport {
    let v = verbatim(path);
    let mut rep = RemoveReport::default();
    match fs::symlink_metadata(&v) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => rep.fail(&v, e),
        Ok(md) if md.file_type().is_symlink() => remove_link(&v, &mut rep),
        Ok(_) => remove_dir_rec(&v, &mut rep),
    }
    rep
}

/// Remove only the link at `path` (never its target).
pub fn remove_link_only(path: &Path) -> RemoveReport {
    let v = verbatim(path);
    let mut rep = RemoveReport::default();
    remove_link(&v, &mut rep);
    rep
}
