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
//! big files tend to slow down the whole process. The explanation is simple, many
//! cloud storage providers do not support concurrent or chunked uploads of a
//! single file, sometimes they would not even support resuming a partial upload.
//! You would need to upload it in one go, sequentially one byte at a time, it's
//! all or nothing.
//!
//! Now consider a scenario, where you upload a huge file, like a mirror of your
//! Raspberry Pi's SD card with the system and configuration on it. I have such a
//! file, it is about 4 GB big. Now, while backing up my system, this was the last
//! file to be uploaded. According to ETA calculations, it would have taken
//! several hours, so I let it run overnight. The next morning I found out that
//! after around 95% of upload process, my internet connection vanished for just a
//! few seconds, but long enough for the transfer tool to abort the upload. The
//! temporary file got deleted from the cloud storage, so I had to start from zero
//! again. Several hours of uploading wasted.
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
//! present them as complete files again.
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
//! ```shell script
//! cargo install scfs
//! ```
//!
//! ## Usage
//!
//! ```text
//! Usage: scfs <COMMAND>
//!
//! Commands:
//!   split  Create a splitting file system
//!   cat    Create a concatenating file system
//!   help   Print this message or the help of the given subcommand(s)
//!
//! Options:
//!   -h, --help     Print help
//!   -V, --version  Print version
//! ```
//!
//! ### SplitFS
//!
//! ```text
//! Usage: scfs split [OPTIONS] <MIRROR> <MOUNTPOINT> [-- <FUSE_OPTIONS_EXTRA>...]
//!
//! Arguments:
//!   <MIRROR>                 Defines the directory that will be mirrored
//!   <MOUNTPOINT>             Defines the mountpoint, where the mirror will be accessible
//!   [FUSE_OPTIONS_EXTRA]...  Additional options, which are passed down to FUSE
//!
//! Options:
//!   -b, --blocksize <BLOCKSIZE>        Sets the desired blocksize [default: 2097152]
//!   -o, --fuse-options <FUSE_OPTIONS>  Additional options, which are passed down to FUSE
//!   -d, --daemon                       Run program in background
//!       --mkdir                        Create mountpoint directory if it does not exist already
//!   -h, --help                         Print help
//!   -V, --version                      Print version
//! ```
//!
//! To mount a directory with SplitFS, use the following form:
//!
//! ```shell script
//! scfs split <base directory> <mount point>
//! ```
//!
//! This can be simplified by using the dedicated `splitfs` binary:
//!
//! ```shell script
//! splitfs <base directory> <mount point>
//! ```
//!
//! The directory specified as `mount point` will now reflect the content of `base
//! directory`, replacing each regular file with a directory that contains
//! enumerated chunks of that file as separate files.
//!
//! It is possible to use a custom block size for the file fragments. For example,
//! to use 1&nbsp;MB chunks instead of the default size of 2&nbsp;MB, you would go
//! with:
//!
//! ```shell script
//! splitfs --blocksize=1048576 <base directory> <mount point>
//! ```
//!
//! Where 1048576 is 1024 * 1024, so one megabyte in bytes.
//!
//! You can even leverage the calculating power of your Shell, like for example in
//! Bash:
//!
//! ```shell script
//! splitfs --blocksize=$((1024 * 1024)) <base directory> <mount point>
//! ```
//!
//! New since v0.9.0: The block size may now also be given with a symbolic
//! quantifier. Allowed quantifiers are "K", "M", "G", and "T", each one
//! multiplying the base with 1024. So, to set the block size to 1&nbsp;MB like in
//! the example above, you can now use:
//!
//! ```shell script
//! splitfs --blocksize=1M <base directory> <mount point>
//! ```
//!
//! You can actually go as far as to set a block size of one byte, but be prepared
//! for a ridiculous amount of overhead or maybe even a system freeze because the
//! metadata table grows too large.
//!
//! ### CatFS
//!
//! ```text
//! Usage: scfs cat [OPTIONS] <MIRROR> <MOUNTPOINT> [-- <FUSE_OPTIONS_EXTRA>...]
//!
//! Arguments:
//!   <MIRROR>                 Defines the directory that will be mirrored
//!   <MOUNTPOINT>             Defines the mountpoint, where the mirror will be accessible
//!   [FUSE_OPTIONS_EXTRA]...  Additional options, which are passed down to FUSE
//!
//! Options:
//!   -o, --fuse-options <FUSE_OPTIONS>  Additional options, which are passed down to FUSE
//!   -d, --daemon                       Run program in background
//!       --mkdir                        Create mountpoint directory if it does not exist already
//!   -h, --help                         Print help
//!   -V, --version                      Print version
//! ```
//!
//! To mount a directory with CatFS, use the following form:
//!
//! ```shell script
//! scfs cat <base directory> <mount point>
//! ```
//!
//! This can be simplified by using the dedicated `catfs` binary:
//!
//! ```shell script
//! catfs <base directory> <mount point>
//! ```
//!
//! Please note that `base directory` needs to be a directory structure that has
//! been generated by SplitFS. CatFS will refuse mounting the directory otherwise.
//!
//! The directory specified as `mount point` will now reflect the content of `base
//! directory`, replacing each directory with chunked files in it as single files.
//!
//! ### Additional FUSE mount options
//!
//! It is possible to pass additional mount options to the underlying FUSE
//! library.
//!
//! SCFS supports two ways of specifying options, either via the "-o" option, or
//! via additional arguments after a "--" separator. This is in accordance to
//! other FUSE based filesystems like EncFS.
//!
//! These two calls are equivalent:
//!
//! ```shell script
//! scfs split -o nonempty mirror mountpoint
//! scfs split mirror mountpoint -- nonempty
//! ```
//!
//! Of course, these methods also work in the `splitfs` and `catfs` binaries.
//!
//! ### Daemon mode
//!
//! Originally, SCFS was meant to be run in the foreground. This proved to be
//! annoying if one wants to use the same terminal for further work. Granted, one
//! could always use features of their Shell to send the process to the
//! background, but then you have a background process that might accidentally be
//! killed if the user closes terminal. Furthermore, SCFS originally did not
//! terminate cleanly if the user unmounted it by external means.
//!
//! Since v0.9.0, SCFS natively supports daemon mode, in that the program changes
//! its working directory to `"/"` and then forks itself into a true daemon
//! process, independent of the running terminal.
//!
//! ```shell script
//! splitfs --daemon mirror mountpoint
//! ```
//!
//! Note that `mirror` and `mountpoint` are resolved *before* changing the working
//! directory, so they can still be given relative to the current working
//! directory.
//!
//! To unmount, `fusermount` can be used:
//!
//! ```shell script
//! fusermount -u mountpoint
//! ```
//!
//! ## Limitations
//!
//! I consider this project no longer a "raw prototype", and I am eating my own
//! dog food, meaning I use it in my own backup strategies and create features
//! based on my personal needs.
//!
//! However, this might not meet the needs of the typical user and without
//! feedback I might not even think of some scenarios to begin with.
//!
//! Specifically, these are the current limitations of SCFS:
//!
//! -   It should work an all UNIX based systems, like Linux and maybe some MacOS
//!     versions, however without MacOS specific file attributes. But definitely
//!     not on Windows, since this would need special handling of system calls,
//!     which I haven't had time to take care of yet.
//!
//! -   It can only work with directories, regular files, and symlinks. Every
//!     other file types (device files, pipes, and so on) will be silently
//!     ignored.
//!
//! -   The base directory will be mounted read-only in the new mount point, and
//!     SCFS expects that the base directory will not be altered while mounted.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::fs::Metadata;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{BackgroundSession, FileAttr, FileType, Filesystem, MountOption};
use rusqlite::Row;
use serde::{Deserialize, Serialize};

pub use cli::Cli;

pub(crate) use catfs::CatFS;
pub(crate) use shared::Shared;
pub(crate) use splitfs::SplitFS;

mod catfs;
mod cli;
mod shared;
mod splitfs;

const TTL: Duration = Duration::from_secs(60 * 60 * 24);

const STMT_CREATE: &str = "
    CREATE TABLE Files (
        ino INTEGER PRIMARY KEY,
        parent_ino INTEGER,
        path TEXT UNIQUE,
        file_name TEXT,
        part INTEGER,
        vdir INTEGER,
        symlink INTEGER
    )
";
const STMT_CREATE_INDEX_PARENT_INO_FILE_NAME: &str = "
    CREATE INDEX idx_parent_ino_file_name
    ON Files (parent_ino, file_name)
";
const STMT_INSERT: &str = "
    INSERT INTO Files (ino, parent_ino, path, file_name, part, vdir, symlink)
    VALUES (?, ?, ?, ?, ?, ?, ?)
";
const STMT_QUERY_BY_INO: &str = "
    SELECT *
    FROM Files
    WHERE ino = ?
";
const STMT_QUERY_BY_PARENT_INO: &str = "
    SELECT *
    FROM Files
    WHERE parent_ino = ?
    LIMIT -1 OFFSET ?
";
const STMT_QUERY_BY_PARENT_INO_AND_FILENAME: &str = "
    SELECT *
    FROM Files
    WHERE parent_ino = ?
    AND file_name = ?
";

const CONFIG_FILE_NAME: &str = ".scfs_config";

const CONFIG_DEFAULT_BLOCKSIZE: u64 = 2 * 1024 * 1024;

const INO_OUTSIDE: u64 = 0;
const INO_ROOT: u64 = 1;
const INO_CONFIG: u64 = 2;

const INO_FIRST_FREE: u64 = 10;

type DropHookFn = Box<dyn Fn() + Send + 'static>;

fn system_time_from_time(secs: i64, nsecs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::new(secs as u64, nsecs as u32)
    } else {
        UNIX_EPOCH - Duration::new((-secs) as u64, nsecs as u32)
    }
}

fn convert_filetype(ft: fs::FileType) -> Option<FileType> {
    if ft.is_dir() {
        Some(FileType::Directory)
    } else if ft.is_file() {
        Some(FileType::RegularFile)
    } else if ft.is_symlink() {
        Some(FileType::Symlink)
    } else {
        None
    }
}

fn convert_metadata_to_attr(meta: Metadata, ino: Option<u64>) -> FileAttr {
    FileAttr {
        ino: if let Some(ino) = ino { ino } else { meta.ino() },
        size: meta.size(),
        blocks: meta.blocks(),
        atime: system_time_from_time(meta.atime(), meta.atime_nsec()),
        mtime: system_time_from_time(meta.mtime(), meta.mtime_nsec()),
        ctime: system_time_from_time(meta.ctime(), meta.ctime_nsec()),
        crtime: meta.created().unwrap_or(system_time_from_time(0, 0)),
        kind: convert_filetype(meta.file_type()).expect("Filetype not supported"),
        perm: meta.mode() as u16,
        nlink: meta.nlink() as u32,
        uid: meta.uid(),
        gid: meta.gid(),
        rdev: meta.rdev() as u32,
        blksize: meta.blksize() as u32,
        flags: 0,
    }
}

// Copied from fuser, mount_options.rs. When this becomes part of their public API, delete this function.
fn mount_option_from_str(s: &str) -> MountOption {
    match s {
        "auto_unmount" => MountOption::AutoUnmount,
        "allow_other" => MountOption::AllowOther,
        "allow_root" => MountOption::AllowRoot,
        "default_permissions" => MountOption::DefaultPermissions,
        "dev" => MountOption::Dev,
        "nodev" => MountOption::NoDev,
        "suid" => MountOption::Suid,
        "nosuid" => MountOption::NoSuid,
        "ro" => MountOption::RO,
        "rw" => MountOption::RW,
        "exec" => MountOption::Exec,
        "noexec" => MountOption::NoExec,
        "atime" => MountOption::Atime,
        "noatime" => MountOption::NoAtime,
        "dirsync" => MountOption::DirSync,
        "sync" => MountOption::Sync,
        "async" => MountOption::Async,
        x if x.starts_with("fsname=") => MountOption::FSName(x[7..].into()),
        x if x.starts_with("subtype=") => MountOption::Subtype(x[8..].into()),
        x => MountOption::CUSTOM(x.into()),
    }
}

fn mount<'a, 'b, FS, P, I>(filesystem: FS, mountpoint: &P, fuse_options: I) -> BackgroundSession
where
    FS: Filesystem + Send + 'static + 'a,
    P: AsRef<Path>,
    I: IntoIterator<Item = &'b OsStr>,
{
    let fuse_options = fuse_options
        .into_iter()
        .map(|x| mount_option_from_str(x.to_str().unwrap()));

    let options = {
        let mut options = vec![MountOption::RO, MountOption::FSName(String::from("scfs"))];
        for opt in fuse_options {
            options.push(opt);
        }
        options
    };

    fuser::spawn_mount2(filesystem, &mountpoint, options.as_ref()).unwrap()
}

struct FileHandle {
    file: OsString,
    start: u64,
    end: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FileInfo {
    ino: u64,
    parent_ino: u64,
    path: OsString,
    file_name: OsString,
    part: u64,
    vdir: bool,
    symlink: bool,
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
            symlink: false,
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
            symlink: false,
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
    path: Vec<u8>,
    file_name: Vec<u8>,
    part: i64,
    vdir: bool,
    symlink: bool,
}

impl From<&Row<'_>> for FileInfoRow {
    fn from(row: &Row) -> Self {
        FileInfoRow {
            ino: row.get_unwrap(0),
            parent_ino: row.get_unwrap(1),
            path: row.get_unwrap(2),
            file_name: row.get_unwrap(3),
            part: row.get_unwrap(4),
            vdir: row.get_unwrap(5),
            symlink: row.get_unwrap(6),
        }
    }
}

impl From<FileInfoRow> for FileInfo {
    fn from(f: FileInfoRow) -> Self {
        FileInfo {
            ino: f.ino as u64,
            parent_ino: f.parent_ino as u64,
            path: OsString::from_vec(f.path),
            file_name: OsString::from_vec(f.file_name),
            part: f.part as u64,
            vdir: f.vdir,
            symlink: f.symlink,
        }
    }
}

impl From<FileInfo> for FileInfoRow {
    fn from(f: FileInfo) -> Self {
        FileInfoRow {
            ino: f.ino as i64,
            parent_ino: f.parent_ino as i64,
            path: f.path.as_bytes().to_vec(),
            file_name: f.file_name.as_bytes().to_vec(),
            part: f.part as i64,
            vdir: f.vdir,
            symlink: f.symlink,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct Config {
    blocksize: u64,
}

impl Config {
    fn blocksize(mut self, blocksize: u64) -> Self {
        self.blocksize = blocksize;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            blocksize: CONFIG_DEFAULT_BLOCKSIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_fileinfo_to_fileinforow_and_back() {
        let file_info = FileInfo {
            ino: 7,
            parent_ino: 3,
            path: OsString::from("File.Name"),
            file_name: OsString::from("/path/to/files/File.Name"),
            part: 5,
            vdir: true,
            symlink: false,
        };

        let file_info_row = FileInfoRow::from(file_info.clone());

        assert_eq!(file_info, file_info_row.into());
    }
}
