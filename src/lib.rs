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
//! In the future, I also want to include the reverse variant, which I would call
//! *CatFS*. If I download such a chunked file, I would `cat * >file` to re-create
//! the actual file. *CatFS* would do this transparently. It would recognize a
//! chunked file and present it as a complete file.
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
//! To mount a directory with SplitFS, use the following form:
//!
//!     scfs <base directory> <mount point>
//!
//! The directory specified as `mount point` will now reflect the content of `base
//! directory`, replacing each regular file with a directory that contains
//! enumerated chunks of that file as separate files.
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
//! -   *CatFS* is de facto not existent by the time of writing. But do not worry,
//!     you can simply manually `cat` all the chunks to retrieve the complete
//!     file.

use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::fs::{File, Metadata};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::os::linux::fs::MetadataExt;
use std::path::Path;

use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request,
};
use libc::ENOENT;
use rusqlite::{params, Connection, Error, Row, NO_PARAMS};
use time::Timespec;

const TTL: Timespec = Timespec {
    sec: 60 * 60 * 24,
    nsec: 0,
};

const STMT_CREATE: &str = "CREATE TABLE Files (
    ino INTEGER PRIMARY KEY,
    parent_ino INTEGER,
    path TEXT UNIQUE,
    part INTEGER
    )";
const STMT_INSERT: &str = "INSERT INTO Files (ino, parent_ino, path, part) VALUES (?, ?, ?, ?)";
const STMT_QUERY_BY_INO: &str = "SELECT * FROM Files WHERE ino = ?";
const STMT_QUERY_BY_PARENT_INO: &str = "SELECT * FROM Files WHERE parent_ino = ? LIMIT -1 OFFSET ?";
const STMT_QUERY_LAST_INO: &str = "SELECT * FROM Files ORDER BY _rowid_ DESC LIMIT 1";

const BLOCK_SIZE: u64 = 2 * 1024 * 1024;

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

fn convert_metadata_to_attr(meta: Metadata, ino: u64) -> FileAttr {
    FileAttr {
        ino: if ino != 0 { ino } else { meta.st_ino() },
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

pub struct SplitFS {
    file_db: Connection,
    file_handles: HashMap<u64, FileHandle>,
}

struct FileHandle {
    file: BufReader<File>,
    start: u64,
    end: u64,
}

#[derive(Debug)]
struct FileInfo {
    ino: u64,
    parent_ino: u64,
    path: OsString,
    part: u64,
}

impl FileInfo {
    fn with_ino(ino: u64) -> Self {
        FileInfo {
            ino,
            parent_ino: Default::default(),
            path: Default::default(),
            part: 0,
        }
    }

    fn with_parent_ino(parent_ino: u64) -> Self {
        FileInfo {
            ino: Default::default(),
            parent_ino,
            path: Default::default(),
            part: 0,
        }
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
    part: i64,
}

impl From<&Row<'_>> for FileInfoRow {
    fn from(row: &Row) -> Self {
        FileInfoRow {
            ino: row.get(0).unwrap(),
            parent_ino: row.get(1).unwrap(),
            path: row.get(2).unwrap(),
            part: row.get(3).unwrap(),
        }
    }
}

impl From<FileInfoRow> for FileInfo {
    fn from(f: FileInfoRow) -> Self {
        FileInfo {
            ino: f.ino as u64,
            parent_ino: f.parent_ino as u64,
            path: OsString::from(f.path),
            part: f.part as u64,
        }
    }
}

impl From<FileInfo> for FileInfoRow {
    fn from(f: FileInfo) -> Self {
        FileInfoRow {
            ino: f.ino as i64,
            parent_ino: f.parent_ino as i64,
            path: f.path.into_string().unwrap(),
            part: f.part as i64,
        }
    }
}

fn populate<P: AsRef<Path>>(file_db: &Connection, path: P, parent_ino: u64) {
    let path = path.as_ref();

    let mut attr = convert_metadata_to_attr(path.metadata().unwrap(), 0);

    attr.ino = if parent_ino == 0 {
        1
    } else {
        file_db
            .prepare_cached(STMT_QUERY_LAST_INO)
            .unwrap()
            .query_map(NO_PARAMS, |row| Ok(FileInfo::from(row).ino))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            + 1
    };

    let file_info = FileInfoRow::from(FileInfo {
        ino: attr.ino,
        parent_ino,
        path: OsString::from(path),
        part: 0,
    });

    file_db
        .prepare_cached(STMT_INSERT)
        .unwrap()
        .execute(params![
            file_info.ino,
            file_info.parent_ino,
            file_info.path,
            file_info.part
        ])
        .unwrap();

    if let FileType::RegularFile = attr.kind {
        let blocks = f64::ceil(attr.size as f64 / BLOCK_SIZE as f64) as u64;
        for i in 0..blocks {
            let file_info = FileInfoRow::from(FileInfo {
                ino: attr.ino + i + 1,
                parent_ino: attr.ino,
                path: OsString::from(path.join(format!("scfs.{:010}", i))),
                part: i + 1,
            });

            file_db
                .prepare_cached(STMT_INSERT)
                .unwrap()
                .execute(params![
                    file_info.ino,
                    file_info.parent_ino,
                    file_info.path,
                    file_info.part
                ])
                .unwrap();
        }
    }

    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            populate(&file_db, entry.path(), attr.ino);
        }
    }
}

impl SplitFS {
    pub fn new(mirror: OsString) -> SplitFS {
        let file_db = Connection::open_in_memory().unwrap();

        file_db.execute(STMT_CREATE, NO_PARAMS).unwrap();

        populate(&file_db, &mirror, 0);

        let file_handles = Default::default();

        SplitFS {
            file_db,
            file_handles,
        }
    }

    fn get_file_info_from_ino(&self, ino: u64) -> Result<FileInfo, Error> {
        let ino = FileInfoRow::from(FileInfo::with_ino(ino)).ino;

        let mut stmt = self.file_db.prepare_cached(STMT_QUERY_BY_INO).unwrap();

        let file_info = stmt
            .query_map(params![ino], |row| Ok(FileInfo::from(row)))?
            .next()
            .unwrap();
        file_info
    }

    fn get_file_info_from_parent_ino_and_file_name(
        &self,
        parent_ino: u64,
        file_name: OsString,
    ) -> Result<FileInfo, Error> {
        let parent_ino = FileInfoRow::from(FileInfo::with_parent_ino(parent_ino)).parent_ino;

        let mut stmt = self
            .file_db
            .prepare_cached(STMT_QUERY_BY_PARENT_INO)
            .unwrap();

        let inos = stmt
            .query_map(params![parent_ino, 0], |row| Ok(FileInfo::from(row).ino))
            .unwrap();

        let file_info = inos
            .map(|ino| {
                let ino = ino.unwrap();
                self.get_file_info_from_ino(ino).unwrap()
            })
            .skip_while(|file_info| {
                let name_from_db = Path::new(&file_info.path).file_name().unwrap();
                let name_to_look_for = Path::new(&file_name).file_name().unwrap();
                name_from_db != name_to_look_for
            })
            .next();

        match file_info {
            Some(f) => Ok(f),
            None => Err(Error::QueryReturnedNoRows),
        }
    }
}

impl Filesystem for SplitFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let file_info =
            self.get_file_info_from_parent_ino_and_file_name(parent, OsString::from(name));
        if let Ok(file_info) = file_info {
            let attr = if file_info.part == 0 {
                let mut meta =
                    convert_metadata_to_attr(fs::metadata(file_info.path).unwrap(), file_info.ino);
                meta.kind = FileType::Directory;
                meta.blocks = 0;
                meta.perm = 0o755;
                meta
            } else {
                let mut meta = convert_metadata_to_attr(
                    fs::metadata(
                        self.get_file_info_from_ino(file_info.parent_ino)
                            .unwrap()
                            .path,
                    )
                    .unwrap(),
                    file_info.ino,
                );
                meta.size = u64::min(BLOCK_SIZE, meta.size - (file_info.part - 1) * BLOCK_SIZE);
                meta
            };
            reply.entry(&TTL, &attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        let file_info = self.get_file_info_from_ino(ino);
        if let Ok(file_info) = file_info {
            let attr = if file_info.part == 0 {
                let mut meta =
                    convert_metadata_to_attr(fs::metadata(file_info.path).unwrap(), file_info.ino);
                meta.kind = FileType::Directory;
                meta.blocks = 0;
                meta.perm = 0o755;
                meta
            } else {
                let mut meta = convert_metadata_to_attr(
                    fs::metadata(
                        self.get_file_info_from_ino(file_info.parent_ino)
                            .unwrap()
                            .path,
                    )
                    .unwrap(),
                    file_info.ino,
                );
                meta.size = u64::min(BLOCK_SIZE, meta.size - (file_info.part - 1) * BLOCK_SIZE);
                meta
            };
            reply.attr(&TTL, &attr)
        } else {
            reply.error(ENOENT)
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        let file_info = self.get_file_info_from_ino(ino);
        if let Ok(file_info) = file_info {
            let file = File::open(
                self.get_file_info_from_ino(file_info.parent_ino)
                    .unwrap()
                    .path,
            )
            .unwrap();
            let mut file = BufReader::new(file);
            let start = (file_info.part - 1) * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;
            file.seek(SeekFrom::Start(start)).unwrap();
            let fh = time::precise_time_ns();
            self.file_handles
                .insert(fh, FileHandle { file, start, end });
            reply.opened(fh, 0);
        } else {
            reply.error(ENOENT)
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        let offset = offset as u64;
        let size = size as u64;

        let handle = self.file_handles.get_mut(&fh).unwrap();

        let offset = offset.min(handle.end - handle.start);
        let size = size.min(handle.end - handle.start - offset);

        handle
            .file
            .seek(SeekFrom::Start(handle.start + offset))
            .unwrap();

        reply.data(
            &handle
                .file
                .borrow_mut()
                .take(size)
                .bytes()
                .map(|b| b.unwrap())
                .collect::<Vec<_>>(),
        );
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.file_handles.remove(&fh);
        reply.ok();
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let file_info = self.get_file_info_from_ino(ino);

        if let Ok(file_info) = file_info {
            let mut stmt = self
                .file_db
                .prepare_cached(STMT_QUERY_BY_PARENT_INO)
                .unwrap();
            let items = stmt
                .query_map(
                    params![
                        FileInfoRow::from(FileInfo::with_parent_ino(file_info.ino)).parent_ino,
                        offset
                    ],
                    |row| Ok(FileInfo::from(row)),
                )
                .unwrap();
            for (off, item) in items.enumerate() {
                let item = item.unwrap();
                reply.add(
                    item.ino,
                    offset + off as i64 + 1,
                    if item.part > 0 {
                        FileType::RegularFile
                    } else {
                        FileType::Directory
                    },
                    Path::new(&item.path).file_name().unwrap(),
                );
            }
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }
}
