//! # SCFS – SplitCatFS
//!
//! A convenient splitting and concatenating filesystem.
//!
//! ## Motivation
//!
//! ### History
//!
//! While setting up a cloud based backup and archive solution, I encountered the
//! following phenomenon: Many small files would get uploaded quite fast and –
//! depending on the actual cloud storage provider – highly concurrently, while
//! big files tend to slow down the whole process. The explanation is simple, most
//! cloud storage providers do not support the upload of a single file, sometimes
//! they would not even support resuming a partial upload. You would need to
//! upload it in one go, sequentially byte for byte, it's all or nothing.
//!
//! Now consider a scenario, where you upload a really big file, like a mirror of
//! your Raspberry Pi's SD card with the system and configuration on it. I have
//! such a file, it is about 4 GB big. Now, while backing up my system, this was
//! the last file to be uploaded. According to ETA calculations, it would have
//! taken several hours, so I let it run overnight. The next morning I found out
//! that after around 95% of upload process, my internet connection vanished for
//! just a few seconds, but long enough that the transfer tool aborted the upload.
//! The temporary file got deleted from the cloud storage, so I had to start from
//! zero again. Several hours of uploading wasted.
//!
//! I thought of a way to split big files, so that I can upload it more
//! efficiently, but I came to the conclusion, that manually splitting files,
//! uploading them, and deleting them afterwards locally, is not a very scalable
//! solution.
//!
//! So I came up with the idea of a special filesystem. A filesystem that would
//! present big files as if they were many small chunks in separate files. In
//! reality, the chunks would all point to the same physical file, only with
//! different offsets. This way I could upload chunked files in parallel without
//! losing too much progress, even if the upload gets aborted midway.
//!
//! *SplitFS* was born.
//!
//! If I download such chunked file parts, I would need to call `cat * >file`
//! afterwards to re-create the actual file. This seems like a similar hassle like
//! manually splitting files. That's why I had also *CatFS* in mind, when
//! developing SCFS. CatFS will concatenate chunked files transparently and
//! present them as a complete files.
//!
//! CatFS is included in SCFS since version 0.4.0.
//!
//!
//! ### Why Rust?
//!
//! I am relatively new to Rust and I thought, the best way to deepen my
//! understanding with Rust is to take on a project that would require dedication
//! and a certain knowledge of the language.
//!
//! ## Installation
//!
//! SCFS can be installed easily through Cargo via `crates.io`:
//!
//!     cargo install scfs
//!
//! ## Usage
//!
//! ### SplitFS
//!
//! To mount a directory with SplitFS, use the following form:
//!
//!     scfs --mode=split <base directory> <mount point>
//!
//! The directory specified as `mount point` will now reflect the content of `base
//! directory`, replacing each regular file with a directory that contains
//! enumerated chunks of that file as separate files.
//!
//! ### CatFS
//!
//! To mount a directory with CatFS, use the following form:
//!
//!     scfs --mode=cat <base directory> <mount point>
//!
//! Please note that `base directory` needs to be a directory structure that has
//! been generated by SplitFS.
//!
//! The directory specified as `mount point` will now reflect the content of `base
//! directory`, replacing each directory with chunked files in it as single files.
//!
//! ## Limitations
//!
//! Please be aware that this project is merely a raw prototype for now!
//! Specifically:
//!
//! -   It only works on Linux for now, maybe even on UNIX. But definitely not on
//!     Windows or MacOS.
//!
//! -   It can only work with directories and regular files. Every other file type
//!     will be ignored or may end end up in a `panic!`.
//!
//! -   The base directory will be mounted read-only in the new mount point and it
//!     is expected that it will not be altered while mounted.
//!
//! -   *CatFS* will not check if `base directory` is a valid SplitFS structure.
//!     This might change in the future. For now, please use CatFS *only* on
//!     directory structures that have been generated by SplitFS.

use std::ffi::OsString;
use std::fs;
use std::fs::Metadata;
use std::os::linux::fs::MetadataExt;

use fuse::{FileAttr, FileType};
use rusqlite::Row;
use serde::{Deserialize, Serialize};
use time::Timespec;

pub use catfs::CatFS;
pub use splitfs::SplitFS;

mod catfs;
mod splitfs;

const TTL: Timespec = Timespec {
    sec: 60 * 60 * 24,
    nsec: 0,
};

const STMT_CREATE: &str = "CREATE TABLE Files (
    ino INTEGER PRIMARY KEY,
    parent_ino INTEGER,
    path TEXT UNIQUE,
    file_name TEXT,
    part INTEGER,
    vdir INTEGER
    )";
const STMT_CREATE_INDEX_PARENT_INO_FILE_NAME: &str =
    "CREATE INDEX idx_parent_ino_file_name ON Files (parent_ino, file_name)";
const STMT_INSERT: &str =
    "INSERT INTO Files (ino, parent_ino, path, file_name, part, vdir) VALUES (?, ?, ?, ?, ?, ?)";
const STMT_QUERY_BY_INO: &str = "SELECT * FROM Files WHERE ino = ?";
const STMT_QUERY_BY_PARENT_INO: &str = "SELECT * FROM Files WHERE parent_ino = ? LIMIT -1 OFFSET ?";
const STMT_QUERY_BY_PARENT_INO_AND_FILENAME: &str =
    "SELECT * FROM Files WHERE parent_ino = ? AND file_name = ?";

const BLOCK_SIZE: u64 = 2 * 1024 * 1024;

const CONFIG_FILE_NAME: &str = ".scfs_config";

const INO_OUTSIDE: u64 = 0;
const INO_ROOT: u64 = 1;
const INO_CONFIG: u64 = 2;

fn convert_filetype(ft: fs::FileType) -> FileType {
    if ft.is_dir() {
        FileType::Directory
    } else if ft.is_file() {
        FileType::RegularFile
    } else if ft.is_symlink() {
        FileType::Symlink
    } else {
        panic!("Not supported")
    }
}

fn convert_metadata_to_attr(meta: Metadata, ino: Option<u64>) -> FileAttr {
    FileAttr {
        ino: if let Some(ino) = ino {
            ino
        } else {
            meta.st_ino()
        },
        size: meta.st_size(),
        blocks: meta.st_blocks(),
        atime: Timespec::new(meta.st_atime(), meta.st_atime_nsec() as i32),
        mtime: Timespec::new(meta.st_mtime(), meta.st_mtime_nsec() as i32),
        ctime: Timespec::new(meta.st_ctime(), meta.st_ctime_nsec() as i32),
        crtime: Timespec::new(0, 0),
        kind: convert_filetype(meta.file_type()),
        perm: meta.st_mode() as u16,
        nlink: meta.st_nlink() as u32,
        uid: meta.st_uid(),
        gid: meta.st_gid(),
        rdev: meta.st_rdev() as u32,
        flags: 0,
    }
}

struct FileHandle {
    file: OsString,
    start: u64,
    end: u64,
}

#[derive(Debug, Default)]
struct FileInfo {
    ino: u64,
    parent_ino: u64,
    path: OsString,
    file_name: OsString,
    part: u64,
    vdir: bool,
}

impl FileInfo {
    fn with_ino(ino: u64) -> Self {
        FileInfo {
            ino,
            parent_ino: Default::default(),
            path: Default::default(),
            file_name: Default::default(),
            part: 0,
            vdir: false,
        }
    }

    fn with_parent_ino(parent_ino: u64) -> Self {
        FileInfo {
            ino: Default::default(),
            parent_ino,
            path: Default::default(),
            file_name: Default::default(),
            part: 0,
            vdir: false,
        }
    }

    fn file_name<S: Into<OsString>>(mut self, file_name: S) -> Self {
        self.file_name = file_name.into();
        self
    }

    fn into_file_info_row(self) -> FileInfoRow {
        FileInfoRow::from(self)
    }
}

impl From<&Row<'_>> for FileInfo {
    fn from(row: &Row) -> Self {
        FileInfoRow::from(row).into()
    }
}

#[derive(Debug)]
struct FileInfoRow {
    ino: i64,
    parent_ino: i64,
    path: String,
    file_name: String,
    part: i64,
    vdir: bool,
}

impl From<&Row<'_>> for FileInfoRow {
    fn from(row: &Row) -> Self {
        FileInfoRow {
            ino: row.get(0).unwrap(),
            parent_ino: row.get(1).unwrap(),
            path: row.get(2).unwrap(),
            file_name: row.get(3).unwrap(),
            part: row.get(4).unwrap(),
            vdir: row.get(5).unwrap(),
        }
    }
}

impl From<FileInfoRow> for FileInfo {
    fn from(f: FileInfoRow) -> Self {
        FileInfo {
            ino: f.ino as u64,
            parent_ino: f.parent_ino as u64,
            path: OsString::from(f.path),
            file_name: OsString::from(f.file_name),
            part: f.part as u64,
            vdir: f.vdir,
        }
    }
}

impl From<FileInfo> for FileInfoRow {
    fn from(f: FileInfo) -> Self {
        FileInfoRow {
            ino: f.ino as i64,
            parent_ino: f.parent_ino as i64,
            path: f.path.into_string().unwrap(),
            file_name: f.file_name.into_string().unwrap(),
            part: f.part as i64,
            vdir: f.vdir,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {}
