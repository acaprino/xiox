//! NTFS Master File Table reader: open `\\.\X:` read-only, ask for the NTFS volume data, read
//! MFT record 0, follow its `$DATA` runs and read the table in 128 KiB chunks, then turn the
//! records into [`Node`]s. Volume access, `fixup`, `data_runs` and the read loop are adapted
//! from the code listed in `VENDORED.md`; the record kept per entry is reduced to what a
//! folder-size index needs.

#![cfg_attr(not(windows), allow(dead_code))]

use crate::{Node, Progress};
use std::sync::atomic::Ordering;

/// MFT read chunk, 0x20000 bytes.
const READ_CHUNK: usize = 131072;
/// Record number of the volume root directory.
const ROOT_RECORD: usize = 5;
/// "No record": dropped records and parents outside the table.
const NONE: u32 = u32::MAX;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Undo the update-sequence protection at the end of every 512-byte stride.
fn fixup(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let usa_off = u16_at(rec, 4) as usize;
    let usa_cnt = u16_at(rec, 6) as usize;
    if usa_cnt == 0 || usa_off + usa_cnt * 2 > rec.len() {
        return false;
    }
    let usn = [rec[usa_off], rec[usa_off + 1]];
    for i in 1..usa_cnt {
        let pos = i * 512 - 2;
        if pos + 2 > rec.len() {
            break;
        }
        if rec[pos..pos + 2] != usn {
            return false;
        }
        rec[pos] = rec[usa_off + i * 2];
        rec[pos + 1] = rec[usa_off + i * 2 + 1];
    }
    true
}

/// Data runs of a non-resident attribute: (first cluster, cluster count); -1 = sparse.
fn data_runs(attr: &[u8]) -> Vec<(i64, u64)> {
    let mut runs = Vec::new();
    let mut p = u16_at(attr, 0x20) as usize;
    let mut lcn: i64 = 0;
    while p < attr.len() {
        let head = attr[p];
        if head == 0 {
            break;
        }
        let len_sz = (head & 0x0f) as usize;
        let off_sz = (head >> 4) as usize;
        p += 1;
        if p + len_sz + off_sz > attr.len() || len_sz == 0 {
            break;
        }
        let mut len: u64 = 0;
        for i in 0..len_sz {
            len |= (attr[p + i] as u64) << (8 * i);
        }
        p += len_sz;
        if off_sz > 0 {
            let mut off: i64 = 0;
            for i in 0..off_sz {
                off |= (attr[p + i] as i64) << (8 * i);
            }
            if attr[p + off_sz - 1] & 0x80 != 0 {
                off |= -1i64 << (8 * off_sz);
            }
            lcn += off;
            runs.push((lcn, len));
        } else {
            runs.push((-1, len));
        }
        p += off_sz;
    }
    runs
}

/// One MFT record, kept small: a name only for folders (file names are never decoded) and
/// the parent as a 32-bit record number. Same size and alignment as [`Node`], so
/// [`into_nodes`] converts the table in place instead of holding two copies.
#[derive(Default)]
struct Rec {
    parent: u32,
    in_use: bool,
    is_dir: bool,
    reparse: bool,
    /// 0 = no `$FILE_NAME` seen; higher = better namespace (Win32 > POSIX > DOS).
    name_rank: u8,
    name: Option<Box<str>>,
    size: u64,
    alloc: u64,
}

const _: () = assert!(
    std::mem::size_of::<Rec>() == std::mem::size_of::<Node>()
        && std::mem::align_of::<Rec>() == std::mem::align_of::<Node>()
);

fn decode_name(a: &[u8], at: usize, units: usize) -> Box<str> {
    char::decode_utf16((0..units).map(|i| u16_at(a, at + i * 2)))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect::<String>()
        .into_boxed_str()
}

fn parse_record(rec: &[u8], no: usize, recs: &mut [Rec]) {
    let flags = u16_at(rec, 0x16);
    let base = (u64_at(rec, 0x20) & 0xFFFF_FFFF_FFFF) as usize;
    let target = if base != 0 { base } else { no };
    if target >= recs.len() {
        return;
    }
    if base == 0 {
        recs[target].in_use = flags & 1 != 0;
        recs[target].is_dir = flags & 2 != 0;
    } else if flags & 1 == 0 {
        return;
    }
    let r = &mut recs[target];
    let mut off = u16_at(rec, 0x14) as usize;
    while off + 16 <= rec.len() {
        let kind = u32_at(rec, off);
        if kind == 0xFFFF_FFFF {
            break;
        }
        let len = u32_at(rec, off + 4) as usize;
        if len < 16 || off + len > rec.len() {
            break;
        }
        let a = &rec[off..off + len];
        let nonres = a[8] != 0;
        let name_len = a[9];
        match kind {
            // $STANDARD_INFORMATION: file attributes.
            0x10 if !nonres => {
                let co = u16_at(a, 0x14) as usize;
                if co + 0x24 <= a.len() && u32_at(a, co + 0x20) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    r.reparse = true;
                }
            }
            // $FILE_NAME: parent and (folders only) the name.
            0x30 if !nonres => {
                let co = u16_at(a, 0x14) as usize;
                if co + 0x42 <= a.len() {
                    let parent = u64_at(a, co) & 0xFFFF_FFFF_FFFF;
                    let nlen = a[co + 0x40] as usize;
                    // Prefer the long name: Win32 (1) or Win32+DOS (3), then POSIX (0), DOS (2) last.
                    let rank = match a[co + 0x41] {
                        1 | 3 => 3,
                        0 => 2,
                        _ => 1,
                    };
                    if rank > r.name_rank && co + 0x42 + nlen * 2 <= a.len() {
                        r.parent = u32::try_from(parent).unwrap_or(NONE);
                        r.name_rank = rank;
                        // An extension record may arrive before its base record, when
                        // `is_dir` is not known yet: keep the name, `into_nodes` drops it.
                        r.name = (r.is_dir || base != 0).then(|| decode_name(a, co + 0x42, nlen));
                    }
                }
            }
            // Unnamed $DATA: logical and allocated size.
            0x80 if name_len == 0 => {
                if nonres {
                    if a.len() >= 0x38 && u64_at(a, 0x10) == 0 {
                        r.alloc = u64_at(a, 0x28);
                        r.size = u64_at(a, 0x30);
                        // Compressed or sparse: the clusters really in use.
                        if u16_at(a, 0x0C) & 0x8001 != 0 && a.len() >= 0x48 {
                            r.alloc = u64_at(a, 0x40);
                        }
                    }
                } else if a.len() >= 0x14 {
                    r.size = u32_at(a, 0x10) as u64;
                    r.alloc = 0; // resident data lives inside the MFT record
                }
            }
            // $REPARSE_POINT
            0xC0 => r.reparse = true,
            _ => {}
        }
        off += len;
    }
}

/// Live records with a `$FILE_NAME` become nodes, renumbered compactly, with the root
/// (record 5) at position 0 and `parent` as a node index (`u32::MAX` when unknown).
fn into_nodes(recs: Vec<Rec>) -> Result<Vec<Node>, String> {
    let total = recs.len();
    if ROOT_RECORD >= total || !recs[ROOT_RECORD].in_use || !recs[ROOT_RECORD].is_dir {
        return Err("radice della MFT non trovata".into());
    }
    let keep = |i: usize, r: &Rec| {
        i == ROOT_RECORD || (r.in_use && if r.is_dir { r.name.is_some() } else { r.name_rank > 0 })
    };
    let mut map = vec![NONE; total];
    let mut kept = 0u32;
    for (i, r) in recs.iter().enumerate() {
        if keep(i, r) {
            map[i] = kept;
            kept += 1;
        }
    }
    // Nodes keep record order; the root then swaps places with the first node.
    let root_at = map[ROOT_RECORD];
    if root_at != 0 {
        let first = map.iter().position(|&m| m == 0).unwrap_or(ROOT_RECORD);
        map[first] = root_at;
        map[ROOT_RECORD] = 0;
    }
    let mut nodes: Vec<Node> = recs
        .into_iter()
        .enumerate()
        .filter(|(i, r)| keep(*i, r))
        .map(|(i, r)| Node {
            parent: if i == ROOT_RECORD { 0 } else { map.get(r.parent as usize).copied().unwrap_or(NONE) },
            is_dir: r.is_dir,
            reparse: r.reparse,
            name: if r.is_dir && i != ROOT_RECORD { r.name } else { None },
            size: if r.is_dir { 0 } else { r.size },
            alloc: if r.is_dir { 0 } else { r.alloc },
        })
        .collect();
    nodes.swap(0, root_at as usize);
    Ok(nodes)
}

#[cfg(windows)]
mod volume {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_BEGIN, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, SetFilePointerEx,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};

    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Volume(HANDLE);

    impl Drop for Volume {
        fn drop(&mut self) {
            // SAFETY: the handle was returned valid by CreateFileW and is closed only here.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Volume {
        /// `\\.\X:`, read-only, sharing read and write. Needs administrator rights.
        pub fn open(letter: char) -> Result<Volume, String> {
            let dev = wide(&format!("\\\\.\\{letter}:"));
            // SAFETY: `dev` is a NUL-terminated UTF-16 string that outlives the call.
            let h = unsafe {
                CreateFileW(
                    dev.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(format!("apertura unità non riuscita ({})", unsafe { GetLastError() }));
            }
            Ok(Volume(h))
        }

        pub fn ntfs_data(&self) -> Result<NTFS_VOLUME_DATA_BUFFER, String> {
            // SAFETY: plain-old-data out-buffer of the size passed to the call.
            let mut vd: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
            let mut ret = 0u32;
            let ok = unsafe {
                DeviceIoControl(
                    self.0,
                    FSCTL_GET_NTFS_VOLUME_DATA,
                    std::ptr::null(),
                    0,
                    (&mut vd as *mut NTFS_VOLUME_DATA_BUFFER).cast(),
                    std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                    &mut ret,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("non è NTFS ({})", unsafe { GetLastError() }));
            }
            Ok(vd)
        }

        pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
            let mut moved = 0i64;
            // SAFETY: valid handle; `moved` is a live out-parameter.
            if unsafe { SetFilePointerEx(self.0, offset as i64, &mut moved, FILE_BEGIN) } == 0 {
                return Err(format!("posizionamento non riuscito ({})", unsafe { GetLastError() }));
            }
            let mut got = 0u32;
            // SAFETY: `buf` is writable for `buf.len()` bytes; synchronous read.
            let ok = unsafe {
                ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut got, std::ptr::null_mut())
            };
            if ok == 0 {
                return Err(format!("lettura non riuscita ({})", unsafe { GetLastError() }));
            }
            Ok(got as usize)
        }
    }
}

/// Read the whole MFT of volume `letter` and return its nodes (root first). Adds every
/// record read to `progress.records`; stops with an error when `progress.cancel` is set.
#[cfg(windows)]
pub fn read_volume(letter: char, progress: &Progress) -> Result<Vec<Node>, String> {
    let vol = volume::Volume::open(letter)?;
    let vd = vol.ntfs_data()?;
    let cluster = vd.BytesPerCluster as u64;
    let rec_size = vd.BytesPerFileRecordSegment as usize;
    if rec_size < 512 || cluster == 0 {
        return Err("dati volume non validi".into());
    }
    // Record 0 describes the MFT itself.
    let first_len = (rec_size as u64).max(cluster).max(vd.BytesPerSector as u64) as usize;
    let mut first = vec![0u8; first_len];
    vol.read_at(vd.MftStartLcn as u64 * cluster, &mut first)?;
    let rec0 = &mut first[..rec_size];
    if !fixup(rec0) {
        return Err("record MFT 0 non valido".into());
    }
    let mut runs = Vec::new();
    let mut mft_size = 0u64;
    let mut off = u16_at(rec0, 0x14) as usize;
    while off + 16 <= rec0.len() {
        let kind = u32_at(rec0, off);
        if kind == 0xFFFF_FFFF {
            break;
        }
        let len = u32_at(rec0, off + 4) as usize;
        if len < 16 || off + len > rec0.len() {
            break;
        }
        let a = &rec0[off..off + len];
        if kind == 0x80 && a[8] != 0 && a[9] == 0 {
            mft_size = u64_at(a, 0x30);
            runs = data_runs(a);
            break;
        }
        off += len;
    }
    if runs.is_empty() || mft_size == 0 {
        return Err("$DATA della MFT non trovato".into());
    }
    let total = (mft_size / rec_size as u64) as usize;
    if total >= NONE as usize {
        return Err("MFT troppo grande".into());
    }
    let mut recs: Vec<Rec> = Vec::new();
    recs.resize_with(total, Rec::default);
    let mut buf = vec![0u8; READ_CHUNK];
    let mut rec_no = 0usize;
    'runs: for (lcn, clusters) in runs {
        let run_bytes = clusters * cluster;
        if lcn < 0 {
            rec_no += (run_bytes / rec_size as u64) as usize;
            continue;
        }
        let mut done = 0u64;
        while done < run_bytes {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err("annullata".into());
            }
            let want = ((run_bytes - done) as usize).min(READ_CHUNK);
            let got = vol.read_at(lcn as u64 * cluster + done, &mut buf[..want])?;
            if got == 0 {
                break;
            }
            let mut p = 0;
            let start = rec_no;
            while p + rec_size <= got {
                if rec_no >= total {
                    progress.records.fetch_add((rec_no - start) as u64, Ordering::Relaxed);
                    break 'runs;
                }
                let r = &mut buf[p..p + rec_size];
                if fixup(r) {
                    parse_record(r, rec_no, &mut recs);
                }
                rec_no += 1;
                p += rec_size;
            }
            progress.records.fetch_add((rec_no - start) as u64, Ordering::Relaxed);
            done += got as u64;
        }
    }
    drop(buf);
    into_nodes(recs)
}

#[cfg(not(windows))]
pub fn read_volume(_letter: char, _progress: &Progress) -> Result<Vec<Node>, String> {
    Err("lettura MFT disponibile solo su Windows".into())
}
