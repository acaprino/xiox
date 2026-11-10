# Vendored code

`src/mft.rs` adapts the NTFS MFT reader of wiztree64 (`albero-disco`, the WizTree clone in
`D:\Projects\KKK\wiztree64\app`, a project without git). The source folder is left untouched.

Copied: 2026-11-10 10:05:05 +0100

| Source file | Size (bytes) | Modified | SHA-256 (`sha256sum`) |
|---|---|---|---|
| `D:\Projects\KKK\wiztree64\app\src\mft.rs` | 11747 | 2026-11-10 09:13:02 +0100 | `f0aeff0c4aa584ba9f18d00609e4d8961504868176aa16d931d946aace7b7996` |
| `D:\Projects\KKK\wiztree64\app\src\sys.rs` | 6278 | 2026-11-10 09:04:13 +0100 | `edb4f2c3e6deb72c21cd3604c72edeb762def39dfb8da6bcd4a0e163a819ca31` |

## What was taken

From `mft.rs`:
- volume access: `\\.\X:` opened read-only (`CreateFileW`, share read/write),
  `FSCTL_GET_NTFS_VOLUME_DATA`, `SetFilePointerEx` + `ReadFile` (`Volume::read_at`);
- `fixup` (update sequence array), `data_runs` (non-resident run list), `u16_at`/`u32_at`/`u64_at`
  — unchanged;
- reading record 0 to find the `$DATA` runs of the MFT, and the read loop in 131072-byte
  chunks (`READ_CHUNK`), sparse runs skipped;
- `parse_record`: base/extension record handling, `$STANDARD_INFORMATION` (0x10),
  `$FILE_NAME` (0x30) with the namespace ranking (Win32 > POSIX > DOS), unnamed `$DATA` (0x80).

From `sys.rs`: only `wide` (UTF-16 + NUL). `is_admin` stays with the caller (`build_for_roots`
takes `admin: bool`); `drives` is not needed.

Not copied: `app.rs`, `treemap.rs`, `fmt.rs`, `model.rs`, `scan.rs`, the rest of `sys.rs`.

## What changed

- `Rec` keeps `in_use`, `is_dir`, `reparse`, `parent: u32`, `name_rank`, `size`, `alloc`;
  `name` is decoded for folders only (40 bytes per record instead of a name per file).
  `modified` and the raw attributes are dropped.
- `reparse` comes from `FILE_ATTRIBUTE_REPARSE_POINT` (0x400) in `$STANDARD_INFORMATION` or
  from a `$REPARSE_POINT` (0xC0) attribute.
- Compressed or sparse unnamed `$DATA`: `alloc` is the total allocated size at offset 0x40
  (clusters really in use) instead of the allocated size at 0x28.
- A `$FILE_NAME` found in an extension record keeps its name even if the base record has
  not been read yet; names of files are dropped in the conversion.
- The table becomes a `Vec<Node>` in place (`Rec` and `Node` have the same size and
  alignment; checked at compile time) with the root (record 5) at position 0; the
  `RawEntry`/`Raw` model and `Progress` of the original are replaced by `dc_disktree::Node`
  and `dc_disktree::Progress` (`records` counter, `cancel` flag).
- windows-sys 0.61 (the version already in this workspace) instead of 0.59.
