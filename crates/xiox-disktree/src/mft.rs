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

/// `N` bytes at `o`, or zeros when the slice is too short: garbage never panics, it only
/// fails the checks that follow.
fn le_bytes<const N: usize>(b: &[u8], o: usize) -> [u8; N] {
    o.checked_add(N)
        .and_then(|e| b.get(o..e))
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0; N])
}
fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(le_bytes(b, o))
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(le_bytes(b, o))
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(le_bytes(b, o))
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

/// Little-endian unsigned integer of 1..=8 bytes.
fn le_uint(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64)
}

/// Data runs of a non-resident attribute (header of 0x40 bytes): (first cluster, cluster
/// count); -1 = sparse. Malformed run lists are an error, never a panic.
fn data_runs(attr: &[u8]) -> Result<Vec<(i64, u64)>, String> {
    const BAD: &str = "invalid segment list";
    if attr.len() < 0x40 {
        return Err(BAD.into());
    }
    let mut runs = Vec::new();
    let mut p = u16_at(attr, 0x20) as usize;
    if p < 0x40 {
        return Err(BAD.into());
    }
    let mut lcn: i64 = 0;
    while let Some(&head) = attr.get(p) {
        if head == 0 {
            break;
        }
        let len_sz = (head & 0x0f) as usize;
        let off_sz = (head >> 4) as usize;
        p += 1;
        if len_sz == 0 || len_sz > 8 || off_sz > 8 || p + len_sz + off_sz > attr.len() {
            return Err(BAD.into());
        }
        let len = le_uint(&attr[p..p + len_sz]);
        p += len_sz;
        if off_sz > 0 {
            // Signed delta from the previous run; an 8-byte field is already full width.
            let mut off = le_uint(&attr[p..p + off_sz]) as i64;
            if off_sz < 8 && attr[p + off_sz - 1] & 0x80 != 0 {
                off |= -1i64 << (8 * off_sz);
            }
            lcn = lcn.checked_add(off).filter(|&l| l >= 0).ok_or(BAD)?;
            runs.push((lcn, len));
        } else {
            runs.push((-1, len));
        }
        p += off_sz;
    }
    Ok(runs)
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

/// Attributes of a fixed-up record, as (type, bytes). Stops at the end marker or at the
/// first attribute whose length is shorter than a resident header (0x18) or overruns the
/// record.
fn attributes(rec: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut off = u16_at(rec, 0x14) as usize;
    std::iter::from_fn(move || {
        let kind = u32_at(rec, off);
        let len = u32_at(rec, off.checked_add(4)?) as usize;
        if off.checked_add(8)? > rec.len() || kind == 0xFFFF_FFFF || len < 0x18 {
            return None;
        }
        let a = rec.get(off..off.checked_add(len)?)?;
        off += len;
        Some((kind, a))
    })
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
    for (kind, a) in attributes(rec) {
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
                    if a.len() >= 0x40 && u64_at(a, 0x10) == 0 {
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
    }
}

/// Live records with a `$FILE_NAME` become nodes, renumbered compactly, with the root
/// (record 5) at position 0 and `parent` as a node index (`u32::MAX` when unknown).
fn into_nodes(recs: Vec<Rec>) -> Result<Vec<Node>, String> {
    let total = recs.len();
    if ROOT_RECORD >= total || !recs[ROOT_RECORD].in_use || !recs[ROOT_RECORD].is_dir {
        return Err("MFT root not found".into());
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

/// What `FSCTL_GET_NTFS_VOLUME_DATA` says about the volume.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    cluster: u64,
    rec_size: usize,
    sector: u64,
    mft_start_lcn: u64,
    /// Valid data length of the MFT, from the file system (records past it were never
    /// written). Caps the record count, which record 0 alone could make arbitrarily large.
    mft_valid: u64,
}

/// Positioned reads from a volume.
trait ReadAt {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, String>;
}

const BAD_VOLUME: &str = "invalid volume data";

/// Fill `buf` from `offset`; anything short of that is an error, not a shorter table.
fn read_exact(vol: &impl ReadAt, offset: u64, buf: &mut [u8]) -> Result<(), String> {
    let got = vol.read_at(offset, buf)?;
    if got != buf.len() {
        return Err(format!("incomplete MFT read ({got} of {} bytes)", buf.len()));
    }
    Ok(())
}

/// Size of the MFT and its data runs, from the unnamed `$DATA` attribute of record 0.
fn mft_data(rec0: &[u8]) -> Result<(u64, Vec<(i64, u64)>), String> {
    for (kind, a) in attributes(rec0) {
        if kind == 0x80 && a[8] != 0 && a[9] == 0 {
            // Non-resident header (0x40 bytes) starting at VCN 0.
            if a.len() < 0x40 || u64_at(a, 0x10) != 0 {
                return Err("invalid MFT $DATA".into());
            }
            return Ok((u64_at(a, 0x30), data_runs(a)?));
        }
    }
    Err("MFT $DATA not found".into())
}

/// Read the MFT described by `g` from `vol`. Errors (never panics, never returns a partial
/// table) on anything it cannot read whole: bad geometry, malformed record 0, an MFT whose
/// runs continue in an attribute list (record 0 alone does not cover it), a run that does
/// not hold whole records, a short read.
fn read_mft(vol: &impl ReadAt, g: Geometry, progress: &Progress) -> Result<Vec<Node>, String> {
    let (cluster, rec_size) = (g.cluster, g.rec_size);
    let rs = rec_size as u64;
    if !cluster.is_power_of_two()
        || cluster > 1 << 21
        || !g.sector.is_power_of_two()
        || g.sector > cluster
        || !rec_size.is_power_of_two()
        || !(512..=READ_CHUNK).contains(&rec_size)
        || g.mft_valid < rs
    {
        return Err(BAD_VOLUME.into());
    }
    // Record 0 describes the MFT itself.
    let mut first = vec![0u8; rec_size.max(cluster as usize)];
    read_exact(vol, g.mft_start_lcn.checked_mul(cluster).ok_or(BAD_VOLUME)?, &mut first)?;
    let rec0 = &mut first[..rec_size];
    if !fixup(rec0) {
        return Err("invalid MFT record 0".into());
    }
    let (mft_size, runs) = mft_data(rec0)?;
    drop(first);

    let total = mft_size.min(g.mft_valid) / rs;
    if total >= NONE as u64 {
        return Err("MFT too large".into());
    }
    let mut covered = 0u64;
    for &(_, clusters) in &runs {
        let run_bytes = clusters.checked_mul(cluster).ok_or(BAD_VOLUME)?;
        if !run_bytes.is_multiple_of(rs) {
            return Err("MFT segment not aligned to the records".into());
        }
        covered = covered.checked_add(run_bytes / rs).ok_or(BAD_VOLUME)?;
    }
    if covered < total {
        return Err(format!("MFT fragmented beyond record 0: the segments cover {covered} records out of {total}"));
    }

    let total = total as usize;
    let mut recs: Vec<Rec> = Vec::new();
    recs.resize_with(total, Rec::default);
    let mut buf = vec![0u8; READ_CHUNK];
    let mut rec_no = 0usize;
    for (lcn, clusters) in runs {
        if rec_no >= total {
            break;
        }
        let run_bytes = clusters * cluster; // checked above
        if lcn < 0 {
            rec_no += (run_bytes / rs) as usize;
            continue;
        }
        let at = (lcn as u64).checked_mul(cluster).ok_or(BAD_VOLUME)?;
        let mut done = 0u64;
        while done < run_bytes && rec_no < total {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            // A multiple of `rec_size`: so are `run_bytes` and READ_CHUNK.
            let want = (run_bytes - done).min(READ_CHUNK as u64) as usize;
            read_exact(vol, at.checked_add(done).ok_or(BAD_VOLUME)?, &mut buf[..want])?;
            let start = rec_no;
            for r in buf[..want].chunks_exact_mut(rec_size) {
                if rec_no >= total {
                    break;
                }
                if fixup(r) {
                    parse_record(r, rec_no, &mut recs);
                }
                rec_no += 1;
            }
            progress.records.fetch_add((rec_no - start) as u64, Ordering::Relaxed);
            done += want as u64;
        }
    }
    drop(buf);
    into_nodes(recs)
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
                return Err(format!("could not open the drive ({})", unsafe { GetLastError() }));
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
                return Err(format!("not NTFS ({})", unsafe { GetLastError() }));
            }
            Ok(vd)
        }
    }

    impl super::ReadAt for Volume {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
            let bad = || super::BAD_VOLUME.to_string();
            let offset = i64::try_from(offset).map_err(|_| bad())?;
            let len = u32::try_from(buf.len()).map_err(|_| bad())?;
            let mut moved = 0i64;
            // SAFETY: valid handle; `moved` is a live out-parameter.
            if unsafe { SetFilePointerEx(self.0, offset, &mut moved, FILE_BEGIN) } == 0 {
                return Err(format!("seek failed ({})", unsafe { GetLastError() }));
            }
            let mut got = 0u32;
            // SAFETY: `buf` is writable for `len` bytes; synchronous read.
            let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), len, &mut got, std::ptr::null_mut()) };
            if ok == 0 {
                return Err(format!("read failed ({})", unsafe { GetLastError() }));
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
    let bad = || BAD_VOLUME.to_string();
    let g = Geometry {
        cluster: vd.BytesPerCluster as u64,
        rec_size: vd.BytesPerFileRecordSegment as usize,
        sector: vd.BytesPerSector as u64,
        mft_start_lcn: u64::try_from(vd.MftStartLcn).map_err(|_| bad())?,
        mft_valid: u64::try_from(vd.MftValidDataLength).map_err(|_| bad())?,
    };
    read_mft(&vol, g, progress)
}

#[cfg(not(windows))]
pub fn read_volume(_letter: char, _progress: &Progress) -> Result<Vec<Node>, String> {
    Err("MFT read is available on Windows only".into())
}

#[cfg(test)]
mod tests {
    //! Handcrafted volumes and records: everything here runs without administrator rights.

    use super::*;
    use crate::DiskIndex;
    use xiox_model::DirIndex;
    use std::path::{Path, PathBuf};

    const REC: usize = 1024;

    fn put16(b: &mut [u8], o: usize, v: u16) {
        b[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(b: &mut [u8], o: usize, v: u64) {
        b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// 1 KiB FILE record (update sequence array at 0x30, 3 entries), attributes from 0x38.
    fn record(flags: u16, base: u64, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut r = vec![0u8; REC];
        r[0..4].copy_from_slice(b"FILE");
        put16(&mut r, 4, 0x30);
        put16(&mut r, 6, 3);
        put16(&mut r, 0x14, 0x38);
        put16(&mut r, 0x16, flags);
        put64(&mut r, 0x20, base);
        let mut off = 0x38;
        for a in attrs {
            r[off..off + a.len()].copy_from_slice(a);
            off += a.len();
        }
        put32(&mut r, off, 0xFFFF_FFFF);
        // Update sequence: number 0xABCD, the real last two bytes of each sector go in the array.
        put16(&mut r, 0x30, 0xABCD);
        for i in 1..3 {
            let pos = i * 512 - 2;
            r[0x30 + i * 2] = r[pos];
            r[0x31 + i * 2] = r[pos + 1];
            put16(&mut r, pos, 0xABCD);
        }
        r
    }
    fn resident(kind: u32, content: &[u8]) -> Vec<u8> {
        let len = (0x18 + content.len()).div_ceil(8) * 8;
        let mut a = vec![0u8; len];
        put32(&mut a, 0, kind);
        put32(&mut a, 4, len as u32);
        put32(&mut a, 0x10, content.len() as u32);
        put16(&mut a, 0x14, 0x18);
        a[0x18..0x18 + content.len()].copy_from_slice(content);
        a
    }
    fn std_info(attrs: u32) -> Vec<u8> {
        let mut c = vec![0u8; 0x48];
        put32(&mut c, 0x20, attrs);
        resident(0x10, &c)
    }
    fn file_name(parent: u64, ns: u8, name: &str) -> Vec<u8> {
        let u: Vec<u16> = name.encode_utf16().collect();
        let mut c = vec![0u8; 0x42 + u.len() * 2];
        put64(&mut c, 0, parent | (7u64 << 48));
        c[0x40] = u.len() as u8;
        c[0x41] = ns;
        for (i, x) in u.iter().enumerate() {
            put16(&mut c, 0x42 + i * 2, *x);
        }
        resident(0x30, &c)
    }
    /// Unnamed non-resident $DATA starting at VCN 0, with the given mapping pairs.
    fn data_nonres(alloc: u64, size: u64, pairs: &[u8]) -> Vec<u8> {
        let len = (0x40 + pairs.len() + 1).div_ceil(8) * 8;
        let mut a = vec![0u8; len];
        put32(&mut a, 0, 0x80);
        put32(&mut a, 4, len as u32);
        a[8] = 1;
        put16(&mut a, 0x20, 0x40);
        put64(&mut a, 0x28, alloc);
        put64(&mut a, 0x30, size);
        put64(&mut a, 0x38, size);
        a[0x40..0x40 + pairs.len()].copy_from_slice(pairs);
        a
    }
    /// Mapping pairs for absolute runs (lcn, clusters), 4-byte fields.
    fn pairs(runs: &[(u64, u64)]) -> Vec<u8> {
        let mut v = Vec::new();
        let mut prev = 0i64;
        for &(lcn, len) in runs {
            v.push(0x44);
            v.extend((len as u32).to_le_bytes());
            v.extend(((lcn as i64 - prev) as i32).to_le_bytes());
            prev = lcn as i64;
        }
        v
    }

    struct Mem {
        bytes: Vec<u8>,
        /// A read starting here returns half of what was asked.
        short_at: Option<u64>,
    }

    impl ReadAt for Mem {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
            let start = usize::try_from(offset).unwrap_or(usize::MAX).min(self.bytes.len());
            let mut n = buf.len().min(self.bytes.len() - start);
            if self.short_at == Some(offset) {
                n /= 2;
            }
            buf[..n].copy_from_slice(&self.bytes[start..start + n]);
            Ok(n)
        }
    }

    fn geometry(cluster: u64, start_lcn: u64, valid_records: u64) -> Geometry {
        Geometry {
            cluster,
            rec_size: REC,
            sector: 512,
            mft_start_lcn: start_lcn,
            mft_valid: valid_records * REC as u64,
        }
    }

    /// A volume whose MFT lies on `runs`; record 0 is the MFT's own record (its $DATA claims
    /// `data_bytes` and lists `runs`, after `extra0`), `records[i]` (when not empty) is
    /// record `i + 1`.
    fn volume(cluster: u64, runs: &[(u64, u64)], data_bytes: u64, extra0: &[Vec<u8>], records: &[Vec<u8>]) -> Mem {
        let alloc = runs.iter().map(|r| r.1).sum::<u64>() * cluster;
        let mut attrs0 = vec![std_info(6)];
        attrs0.extend(extra0.iter().cloned());
        attrs0.push(file_name(5, 3, "$MFT"));
        attrs0.push(data_nonres(alloc, data_bytes, &pairs(runs)));
        let mut all = vec![record(1, 0, &attrs0)];
        all.extend(records.iter().cloned());
        let end = runs.iter().map(|&(l, c)| l + c).max().unwrap();
        let mut bytes = vec![0u8; (end * cluster) as usize];
        let mut no = 0;
        for &(lcn, c) in runs {
            let (mut at, stop) = ((lcn * cluster) as usize, ((lcn + c) * cluster) as usize);
            while at + REC <= stop {
                if let Some(r) = all.get(no).filter(|r| r.len() == REC) {
                    bytes[at..at + REC].copy_from_slice(r);
                }
                no += 1;
                at += REC;
            }
        }
        Mem { bytes, short_at: None }
    }

    /// Records 1..=15: 5 root, 6 Projects, 7 target (in 6), 8 file 5000/8192 in 7,
    /// 9 resident file of 10 bytes in 7, 10 junction "link" in 6; the rest unused.
    fn sample_records() -> Vec<Vec<u8>> {
        let mut v = vec![Vec::new(); 15];
        v[4] = record(3, 0, &[std_info(0x16), file_name(5, 3, ".")]);
        v[5] = record(3, 0, &[std_info(0x10), file_name(5, 1, "Projects")]);
        v[6] = record(3, 0, &[std_info(0x10), file_name(6, 1, "target")]);
        v[7] = record(1, 0, &[std_info(0x20), file_name(7, 1, "a.o"), data_nonres(8192, 5000, &[])]);
        v[8] = record(1, 0, &[std_info(0x20), file_name(7, 1, "b"), resident(0x80, &[7; 10])]);
        v[9] = record(3, 0, &[std_info(0x410), file_name(6, 1, "link")]);
        v
    }

    fn sizes(ix: &DiskIndex, p: &str) -> Option<(u64, u64, u64)> {
        ix.size_of(Path::new(p)).map(|s| (s.logical_bytes, s.allocated_bytes.unwrap(), s.file_count))
    }

    #[test]
    fn reads_contiguous_and_fragmented_mft_from_a_handcrafted_volume() {
        for runs in [vec![(4, 4)], vec![(4, 2), (10, 2)]] {
            let vol = volume(4096, &runs, 16 * REC as u64, &[], &sample_records());
            let progress = Progress::default();
            let nodes = read_mft(&vol, geometry(4096, 4, 16), &progress).unwrap();
            assert_eq!(progress.records.load(Ordering::Relaxed), 16, "{runs:?}");
            let ix = DiskIndex::from_nodes(r"X:\", nodes);
            assert_eq!(sizes(&ix, r"X:\Projects\target"), Some((5010, 8192, 2)), "{runs:?}");
            assert_eq!(sizes(&ix, r"X:\Projects"), Some((5010, 8192, 2)));
            // + $MFT itself: 16 records, 4 clusters.
            assert_eq!(sizes(&ix, r"X:\"), Some((5010 + 16384, 8192 + 16384, 3)));
            assert_eq!(sizes(&ix, r"X:\Projects\link"), None, "junction");
            let hits = ix.find_dirs_named(&["target", "link"], &[PathBuf::from(r"X:\")]);
            assert_eq!(hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(), vec![PathBuf::from(r"X:\Projects\target")]);
        }
    }

    #[test]
    fn mft_data_size_is_capped_by_the_valid_length() {
        let vol = volume(4096, &[(4, 4)], u64::MAX, &[], &sample_records());
        let nodes = read_mft(&vol, geometry(4096, 4, 16), &Progress::default()).unwrap();
        let ix = DiskIndex::from_nodes(r"X:\", nodes);
        assert_eq!(sizes(&ix, r"X:\Projects\target"), Some((5010, 8192, 2)));
        // The cap is the valid length, not the claimed size: 32 valid records, 16 on disk.
        let err = read_mft(&vol, geometry(4096, 4, 32), &Progress::default()).unwrap_err();
        assert!(err.contains("fragmented"), "{err}");
    }

    #[test]
    fn mft_continued_in_an_attribute_list_is_an_error() {
        // Record 0 has an $ATTRIBUTE_LIST and its $DATA lists only the first 8 of 16 records.
        let list = resident(0x20, &[0u8; 64]);
        let vol = volume(4096, &[(4, 2)], 16 * REC as u64, &[list], &sample_records());
        let err = read_mft(&vol, geometry(4096, 4, 16), &Progress::default()).unwrap_err();
        assert_eq!(err, "MFT fragmented beyond record 0: the segments cover 8 records out of 16");
    }

    #[test]
    fn short_read_is_an_error() {
        let mut vol = volume(4096, &[(4, 2), (10, 2)], 16 * REC as u64, &[], &sample_records());
        vol.short_at = Some(10 * 4096);
        let err = read_mft(&vol, geometry(4096, 4, 16), &Progress::default()).unwrap_err();
        assert_eq!(err, "incomplete MFT read (4096 of 8192 bytes)");
        vol.short_at = Some(4 * 4096);
        let err = read_mft(&vol, geometry(4096, 4, 16), &Progress::default()).unwrap_err();
        assert!(err.starts_with("incomplete MFT read"), "{err}");
        // Past the end of the volume.
        let vol = volume(4096, &[(4, 2), (10, 2)], 16 * REC as u64, &[], &sample_records());
        let err = read_mft(&vol, geometry(4096, 40, 16), &Progress::default()).unwrap_err();
        assert!(err.starts_with("incomplete MFT read"), "{err}");
    }

    #[test]
    fn run_that_does_not_hold_whole_records_is_an_error() {
        // 512-byte clusters, 1 KiB records: a run of 3 clusters splits a record in two.
        let vol = volume(512, &[(8, 3), (20, 29)], 16 * REC as u64, &[], &sample_records());
        let err = read_mft(&vol, geometry(512, 8, 16), &Progress::default()).unwrap_err();
        assert_eq!(err, "MFT segment not aligned to the records");
        // Same layout with whole records per run reads fine.
        let vol = volume(512, &[(8, 4), (20, 28)], 16 * REC as u64, &[], &sample_records());
        assert!(read_mft(&vol, geometry(512, 8, 16), &Progress::default()).is_ok());
    }

    #[test]
    fn bad_geometry_and_overflowing_arithmetic_are_errors() {
        let vol = volume(4096, &[(4, 4)], 16 * REC as u64, &[], &sample_records());
        let p = Progress::default();
        for g in [
            Geometry { cluster: 3000, ..geometry(4096, 4, 16) },
            Geometry { cluster: 0, ..geometry(4096, 4, 16) },
            Geometry { cluster: 1 << 22, ..geometry(4096, 4, 16) },
            Geometry { rec_size: 1000, ..geometry(4096, 4, 16) },
            Geometry { rec_size: 256, ..geometry(4096, 4, 16) },
            Geometry { sector: 0, ..geometry(4096, 4, 16) },
            Geometry { sector: 8192, ..geometry(4096, 4, 16) },
            geometry(4096, 4, 0),
            geometry(4096, u64::MAX / 2, 16),
        ] {
            assert_eq!(read_mft(&vol, g, &p).unwrap_err(), BAD_VOLUME, "{g:?}");
        }
        // A run of 2^64-1 clusters: clusters * cluster overflows.
        let mut rec0_pairs = vec![0x48u8];
        rec0_pairs.extend(u64::MAX.to_le_bytes());
        rec0_pairs.extend(4u32.to_le_bytes());
        let rec0 = record(1, 0, &[file_name(5, 3, "$MFT"), data_nonres(0, 16 * REC as u64, &rec0_pairs)]);
        let mut vol = volume(4096, &[(4, 4)], 16 * REC as u64, &[], &sample_records());
        vol.bytes[4 * 4096..4 * 4096 + REC].copy_from_slice(&rec0);
        assert_eq!(read_mft(&vol, geometry(4096, 4, 16), &p).unwrap_err(), BAD_VOLUME);
    }

    #[test]
    fn malformed_record_zero_is_an_error() {
        let p = Progress::default();
        let with_rec0 = |rec0: Vec<u8>| {
            let mut vol = volume(4096, &[(4, 4)], 16 * REC as u64, &[], &sample_records());
            vol.bytes[4 * 4096..4 * 4096 + REC].copy_from_slice(&rec0);
            read_mft(&vol, geometry(4096, 4, 16), &p).unwrap_err()
        };
        // Non-resident $DATA shorter than its 0x40-byte header.
        let mut short = vec![0u8; 0x38];
        put32(&mut short, 0, 0x80);
        put32(&mut short, 4, 0x38);
        short[8] = 1;
        put64(&mut short, 0x30, 1 << 40);
        assert_eq!(with_rec0(record(1, 0, &[short])), "invalid MFT $DATA");
        // $DATA that does not start at VCN 0 (a later piece of the run list).
        let mut later = data_nonres(16384, 16384, &pairs(&[(4, 4)]));
        put64(&mut later, 0x10, 4);
        assert_eq!(with_rec0(record(1, 0, &[later])), "invalid MFT $DATA");
        assert_eq!(with_rec0(record(1, 0, &[std_info(6)])), "MFT $DATA not found");
        assert_eq!(with_rec0(vec![0u8; REC]), "invalid MFT record 0");
        // Run list with a 9-byte length field.
        let bad = data_nonres(16384, 16384, &[0x19, 1, 2, 3, 4, 5, 6, 7, 8, 9, 4]);
        assert_eq!(with_rec0(record(1, 0, &[bad])), "invalid segment list");
    }

    #[test]
    fn data_runs_reject_malformed_lists() {
        let attr = |pairs: &[u8]| data_nonres(0, 0, pairs);
        let bad = |a: &[u8]| data_runs(a).unwrap_err();
        assert_eq!(data_runs(&attr(&[0x11, 4, 10, 0x01, 2, 0x11, 1, 0xFE])).unwrap(), vec![(10, 4), (-1, 2), (8, 1)]);
        bad(&attr(&[])[..0x3F]); // shorter than the header
        let mut low = attr(&[0x11, 4, 10]);
        put16(&mut low, 0x20, 0x18); // run list inside the header
        bad(&low);
        bad(&attr(&[0x09, 1, 2, 3, 4, 5, 6, 7, 8, 9])); // 9-byte length
        bad(&attr(&[0x91, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9])); // 9-byte offset
        bad(&attr(&[0x10, 1])); // zero-byte length
        bad(&attr(&[0x44, 1, 0, 0, 0, 5])); // fields run past the attribute (0x48 bytes)
        bad(&attr(&[0x11, 1, 0xFF])); // first cluster -1
        // 8-byte offsets are full width (no 1 << 64 sign extension).
        let mut wide = vec![0x81, 1];
        wide.extend(100i64.to_le_bytes());
        wide.extend([0x81, 1]);
        wide.extend((-50i64).to_le_bytes());
        assert_eq!(data_runs(&attr(&wide)).unwrap(), vec![(100, 1), (50, 1)]);
        let mut over = vec![0x81, 1];
        over.extend(i64::MAX.to_le_bytes());
        over.extend([0x11, 1, 1]);
        bad(&attr(&over)); // lcn overflows
    }

    #[test]
    fn attributes_shorter_than_a_header_are_ignored() {
        for kind in [0x10u32, 0x30, 0x80, 0xC0] {
            for len in 16..0x18u32 {
                // Content offset 0xFFFF: reading it from a 16..23-byte attribute used to panic.
                let mut a = vec![0xFFu8; len as usize];
                put32(&mut a, 0, kind);
                put32(&mut a, 4, len);
                a[8] = 0;
                let r = record(3, 0, &[a, file_name(5, 1, "after")]);
                let mut recs: Vec<Rec> = Vec::new();
                recs.resize_with(8, Rec::default);
                parse_record(&r, 7, &mut recs);
                let r7 = &recs[7];
                assert!(r7.in_use && !r7.reparse && r7.name.is_none() && r7.size == 0, "{kind:#x} {len}");
            }
        }
        // First attribute offset at the very end, or past it.
        for at in [REC as u16 - 4, REC as u16 - 1, 0xFFFF] {
            let mut r = record(3, 0, &[file_name(5, 1, "x")]);
            put16(&mut r, 0x14, at);
            let mut recs: Vec<Rec> = Vec::new();
            recs.resize_with(8, Rec::default);
            parse_record(&r, 7, &mut recs);
            assert!(recs[7].name.is_none());
        }
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    /// A FILE record with a valid update sequence and random attributes of real types.
    fn random_record(rng: &mut Rng) -> Vec<u8> {
        let mut attrs = Vec::new();
        let mut used = 0x38;
        for _ in 0..rng.below(7) {
            let kind = [0x10u32, 0x20, 0x30, 0x80, 0xC0, 0xFFFF_FFFF][rng.below(6) as usize];
            let len = match rng.below(5) {
                0 => rng.below(0x18) as usize,
                1 => rng.below(4096) as usize,
                _ => 0x18 + rng.below(0x90) as usize,
            };
            let mut a = rng.bytes(len.clamp(8, 0xA8));
            put32(&mut a, 0, kind);
            put32(&mut a, 4, len as u32);
            if rng.below(2) == 0 && a.len() >= 0x16 {
                put16(&mut a, 0x14, rng.below(0x60) as u16);
            }
            if used + a.len() > REC - 0x40 {
                break;
            }
            used += a.len();
            attrs.push(a);
        }
        let base = if rng.below(4) == 0 { rng.below(80) } else { 0 };
        let mut r = record(rng.below(4) as u16, base, &attrs);
        if rng.below(8) == 0 {
            put16(&mut r, 0x14, rng.below(0x480) as u16);
        }
        r
    }

    #[test]
    fn random_records_and_run_lists_never_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        // Records -> nodes -> index.
        for _ in 0..200 {
            let mut recs: Vec<Rec> = Vec::new();
            recs.resize_with(64, Rec::default);
            for no in 0..64 {
                let mut r = random_record(&mut rng);
                if rng.below(3) == 0 {
                    r = record(3, 0, &[std_info(rng.next() as u32), file_name(rng.below(70), rng.below(4) as u8, "d")]);
                }
                if fixup(&mut r) {
                    parse_record(&r, no, &mut recs);
                }
            }
            if let Ok(nodes) = into_nodes(recs) {
                let n = nodes.len() as u32;
                assert!(nodes.iter().all(|x| x.parent < n || x.parent == NONE));
                let ix = DiskIndex::from_nodes(r"X:\", nodes);
                let _ = ix.find_dirs_named(&["d"], &[PathBuf::from(r"X:\")]);
            }
        }
        // Run lists.
        for _ in 0..5000 {
            let n = rng.below(40) as usize;
            let mut a = data_nonres(0, 0, &rng.bytes(n));
            if rng.below(8) == 0 {
                put16(&mut a, 0x20, rng.next() as u16);
            }
            let _ = data_runs(&a);
        }
        // Whole reads with a random record 0 over random volume bytes.
        for _ in 0..500 {
            let (alloc, size, n) = (rng.next(), rng.below(1 << 20), rng.below(32) as usize);
            let mut rec0_data = data_nonres(alloc, size, &rng.bytes(n));
            if rng.below(4) == 0 {
                put64(&mut rec0_data, 0x10, rng.below(3));
            }
            let rec0 = record(1, 0, &[std_info(6), rec0_data]);
            let mut vol = Mem { bytes: rng.bytes(16 * 4096), short_at: None };
            let start = rng.below(16);
            vol.bytes[(start * 4096) as usize..(start * 4096) as usize + REC].copy_from_slice(&rec0);
            for i in 0..8 {
                let at = rng.below(16 * 4) as usize * REC;
                if at != (start * 4096) as usize {
                    let r = if i % 2 == 0 { random_record(&mut rng) } else { sample_records()[rng.below(15) as usize].clone() };
                    if r.len() == REC {
                        vol.bytes[at..at + REC].copy_from_slice(&r);
                    }
                }
            }
            let _ = read_mft(&vol, geometry(4096, start, rng.below(80)), &Progress::default());
        }
    }
}
