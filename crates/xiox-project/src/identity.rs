//! The identity of a folder on disk, so a removal can tell the folder it was confirmed for from
//! another one made later at the same path.

use std::path::Path;

/// Volume serial and 128-bit file id of a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FolderId {
    volume: u64,
    file: [u8; 16],
}

/// The identity of the folder at `path` itself: a link is opened, never followed. `None` when it
/// cannot be read, and on systems without file ids.
pub(crate) fn folder_id(path: &Path) -> Option<FolderId> {
    sys::folder_id(path)
}

#[cfg(windows)]
mod sys {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandleEx,
    };

    use super::FolderId;

    pub(super) fn folder_id(path: &Path) -> Option<FolderId> {
        let f = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .ok()?;
        // SAFETY: the handle is owned by `f` and alive for the call; the buffer is a correctly
        // sized plain-old-data struct for `FileIdInfo`.
        unsafe {
            let mut info: FILE_ID_INFO = std::mem::zeroed();
            let ok = GetFileInformationByHandleEx(
                f.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            );
            (ok != 0).then_some(FolderId { volume: info.VolumeSerialNumber, file: info.FileId.Identifier })
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::Path;

    use super::FolderId;

    pub(super) fn folder_id(_path: &Path) -> Option<FolderId> {
        None
    }
}
