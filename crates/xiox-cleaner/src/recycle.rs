//! Is the Recycle Bin big enough? A move to the bin that does not fit makes Windows ask, per
//! file, whether to delete permanently (or silently purge older items): the UI asks first.
//!
//! [`recycle_room`] reads the free room of one volume's bin (quota minus used), [`shortfall`]
//! compares it with what a plan would move there. Unknown room is never a warning.

/// Free room in the Recycle Bin of `volume` (a drive letter), in bytes: the quota
/// (`MaxCapacity` under `HKCU\…\Explorer\BitBucket\Volume\{GUID}`, in MB) minus what the bin
/// already holds (`SHQueryRecycleBinW`). `Some(0)` when the bin is off for that volume
/// (`NukeOnDelete = 1`). `None` when it cannot be known: not Windows, not a drive letter, no
/// volume GUID, no registry value (the Windows default is not guessed).
pub fn recycle_room(volume: char) -> Option<u64> {
    #[cfg(windows)]
    {
        win::recycle_room(volume)
    }
    #[cfg(not(windows))]
    {
        let _ = volume;
        None
    }
}

/// The volumes whose bin is too small: `(volume, needed, available)` for each entry of `needed`
/// whose `room` is known and smaller than what it needs. Unknown room (`None`): no entry.
pub fn shortfall(needed: &[(char, u64)], room: impl Fn(char) -> Option<u64>) -> Vec<(char, u64, u64)> {
    needed
        .iter()
        .filter_map(|&(volume, need)| match room(volume) {
            Some(have) if need > have => Some((volume, need, have)),
            _ => None,
        })
        .collect()
}

/// `{GUID}` out of a volume name as `GetVolumeNameForVolumeMountPointW` returns it
/// (`\\?\Volume{GUID}\`), braces included: the name of the volume's `BitBucket` key.
#[cfg_attr(not(windows), allow(dead_code))]
fn volume_guid(volume_name: &str) -> Option<&str> {
    let rest = volume_name.strip_prefix(r"\\?\Volume")?;
    let end = rest.find('}')?;
    let guid = &rest[..=end];
    (guid.starts_with('{') && guid.len() > 2).then_some(guid)
}

#[cfg(windows)]
mod win {
    use super::volume_guid;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    use windows_sys::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const BITBUCKET: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume";

    pub fn recycle_room(volume: char) -> Option<u64> {
        if !volume.is_ascii_alphabetic() {
            return None;
        }
        let root: Vec<u16> = format!("{}:\\", volume.to_ascii_uppercase()).encode_utf16().chain([0]).collect();

        let mut name = [0u16; 64];
        // SAFETY: `root` is NUL-terminated, `name` is a writable buffer of the length given.
        let ok = unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), name.as_mut_ptr(), name.len() as u32) };
        if ok == 0 {
            return None;
        }
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let name = String::from_utf16_lossy(&name[..len]);
        let guid = volume_guid(&name)?;

        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{BITBUCKET}\{guid}")).ok()?;
        if key.get_value::<u32, _>("NukeOnDelete").ok() == Some(1) {
            return Some(0);
        }
        let quota = u64::from(key.get_value::<u32, _>("MaxCapacity").ok()?).saturating_mul(1024 * 1024);

        let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
        // SAFETY: `root` is NUL-terminated, `info` is a valid SHQUERYRBINFO with `cbSize` set.
        let hr = unsafe { SHQueryRecycleBinW(root.as_ptr(), &mut info) };
        if hr < 0 {
            return None;
        }
        let used = u64::try_from(info.i64Size).unwrap_or(0);
        Some(quota.saturating_sub(used))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn recycle_bin_too_small_is_reported() {
        assert_eq!(shortfall(&[('C', 10 * GB)], |_| Some(2 * GB)), vec![('C', 10 * GB, 2 * GB)]);
    }

    #[test]
    fn recycle_bin_big_enough_is_fine() {
        assert!(shortfall(&[('C', 10 * GB)], |_| Some(20 * GB)).is_empty());
    }

    #[test]
    fn recycle_bin_unknown_room_gives_no_warning() {
        assert!(shortfall(&[('C', 10 * GB)], |_| None).is_empty());
    }

    #[test]
    fn recycle_bin_two_volumes_only_the_tight_one() {
        let room = |v| match v {
            'C' => Some(50 * GB),
            'D' => Some(GB),
            _ => None,
        };
        assert_eq!(shortfall(&[('C', 10 * GB), ('D', 3 * GB)], room), vec![('D', 3 * GB, GB)]);
    }

    #[test]
    fn recycle_volume_guid_from_volume_name() {
        assert_eq!(
            volume_guid(r"\\?\Volume{0a1b2c3d-1111-2222-3333-444455556666}\"),
            Some("{0a1b2c3d-1111-2222-3333-444455556666}")
        );
        assert_eq!(volume_guid(r"\\?\Volume{}\"), None);
        assert_eq!(volume_guid(r"C:\"), None);
        assert_eq!(volume_guid(r"\\?\Volume{broken"), None);
    }
}
