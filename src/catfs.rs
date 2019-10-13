use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::{fs, thread};

use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request,
};
use libc::ENOENT;
use rusqlite::{params, Connection, Error, NO_PARAMS};

use crate::{
    convert_metadata_to_attr, Config, FileHandle, FileInfo, FileInfoRow, BLOCK_SIZE,
    CONFIG_FILE_NAME, INO_OUTSIDE, INO_ROOT, STMT_CREATE, STMT_INSERT, STMT_QUERY_BY_INO,
    STMT_QUERY_BY_PARENT_INO, TTL,
};

pub struct CatFS {
    file_db: Connection,
    file_handles: HashMap<u64, Vec<FileHandle>>,
    config: Config,
}

impl CatFS {
    pub fn new(mirror: &OsStr) -> Self {
        let config = serde_json::from_str(
            &fs::read_to_string(Path::new(&mirror).join(CONFIG_FILE_NAME))
                .expect("SCFS config file not found"),
        )
        .expect("SCFS config file contains invalid JSON");

        let file_db = Connection::open_in_memory().unwrap();

        file_db.execute(STMT_CREATE, NO_PARAMS).unwrap();

        CatFS::populate(&file_db, &mirror, INO_OUTSIDE);

        {
            let query = "UPDATE Files SET vdir = 1
                 WHERE ino IN (
                    SELECT DISTINCT parent_ino FROM Files WHERE part != 0
                )";
            let mut stmt = file_db.prepare(query).unwrap();
            stmt.execute(NO_PARAMS).unwrap();
        }

        let file_handles = Default::default();

        CatFS {
            file_db,
            file_handles,
            config,
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

    fn get_files_info_from_parent_ino(&self, parent_ino: u64) -> Vec<FileInfo> {
        let parent_ino = FileInfoRow::from(FileInfo::with_parent_ino(parent_ino)).parent_ino;

        let mut stmt = self
            .file_db
            .prepare_cached(STMT_QUERY_BY_PARENT_INO)
            .unwrap();

        stmt.query_map(params![parent_ino, 0], |row| Ok(FileInfo::from(row)))
            .unwrap()
            .map(|res| res.unwrap())
            .collect()
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

    fn get_attr_from_file_info(&self, file_info: &FileInfo) -> FileAttr {
        if file_info.vdir {
            let parts = self.get_files_info_from_parent_ino(file_info.ino);
            let attrs = parts
                .iter()
                .map(|info| {
                    convert_metadata_to_attr(fs::metadata(&info.path).unwrap(), Some(info.ino))
                })
                .collect::<Vec<_>>();
            let mut attr = attrs.get(0).unwrap().clone();
            attr.ino = file_info.ino;
            attr.blocks = attrs.iter().map(|attr| attr.blocks).sum();
            attr.size = attrs.iter().map(|attr| attr.size).sum();
            attr
        } else {
            let attr = convert_metadata_to_attr(
                fs::metadata(&file_info.path).unwrap(),
                Some(file_info.ino),
            );
            attr
        }
    }

    fn populate<P: AsRef<Path>>(file_db: &Connection, path: P, parent_ino: u64) {
        let path = path.as_ref();

        if path.file_name().unwrap() == CONFIG_FILE_NAME {
            return;
        }

        let ino = if parent_ino == INO_OUTSIDE {
            INO_ROOT
        } else {
            time::precise_time_ns()
        };

        let file_info = FileInfoRow::from(FileInfo {
            ino,
            parent_ino,
            path: OsString::from(path),
            part: if path.is_file() {
                path.file_name().unwrap().to_str().unwrap()[5..]
                    .parse::<u64>()
                    .unwrap()
                    + 1
            } else {
                0
            },
            vdir: false,
        });

        file_db
            .prepare_cached(STMT_INSERT)
            .unwrap()
            .execute(params![
                file_info.ino,
                file_info.parent_ino,
                file_info.path,
                file_info.part,
                file_info.vdir
            ])
            .unwrap();

        if path.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                CatFS::populate(&file_db, entry.path(), ino);
            }
        }
    }
}

impl Filesystem for CatFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let file_info =
            self.get_file_info_from_parent_ino_and_file_name(parent, OsString::from(name));
        if let Ok(file_info) = file_info {
            let attr = self.get_attr_from_file_info(&file_info);
            reply.entry(&TTL, &attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        let file_info = self.get_file_info_from_ino(ino);
        if let Ok(file_info) = file_info {
            let attr = self.get_attr_from_file_info(&file_info);
            reply.attr(&TTL, &attr)
        } else {
            reply.error(ENOENT)
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        let files = self.get_files_info_from_parent_ino(ino);

        let fhs = files
            .iter()
            .map(|file| FileHandle {
                file: file.path.clone(),
                start: 0,
                end: 0,
            })
            .collect();

        let fh = time::precise_time_ns();
        self.file_handles.insert(fh, fhs);
        reply.opened(fh, 0);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        let offset = offset as usize;
        let size = size as usize;

        let file_size = self
            .get_attr_from_file_info(&self.get_file_info_from_ino(ino).unwrap())
            .size as usize;

        let offset = offset.min(file_size);
        let size = size.min(file_size - offset);

        let part_start = offset / BLOCK_SIZE as usize;
        let part_end = (offset + size) / BLOCK_SIZE as usize;

        let files = (part_start..=part_end)
            .map(|part| {
                self.file_handles
                    .get(&fh)
                    .unwrap()
                    .get(part)
                    .unwrap()
                    .file
                    .clone()
            })
            .collect::<Vec<_>>();

        thread::spawn(move || {
            let part_start = 0;
            let part_end = files.len() - 1;

            let bytes = files
                .iter()
                .enumerate()
                .map(|(part, file)| {
                    let mut file = BufReader::new(File::open(file).unwrap());

                    if part == part_start {
                        file.seek(SeekFrom::Start(offset as u64 % BLOCK_SIZE))
                            .unwrap();
                    } else {
                        file.seek(SeekFrom::Start(0)).unwrap();
                    }

                    let bytes = file.bytes().map(|b| b.unwrap());

                    bytes
                        .take(if part == part_end {
                            (if part_start == part_end { 0 } else { offset } + size)
                                % BLOCK_SIZE as usize
                        } else {
                            BLOCK_SIZE as usize
                        })
                        .collect::<Vec<_>>()
                })
                .flatten()
                .collect::<Vec<_>>();

            reply.data(&bytes);
        });
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
            if offset < 2 {
                if offset == 0 {
                    reply.add(file_info.ino, 1, FileType::Directory, ".");
                }
                reply.add(
                    if file_info.parent_ino == INO_OUTSIDE {
                        file_info.ino
                    } else {
                        file_info.parent_ino
                    },
                    2,
                    FileType::Directory,
                    "..",
                );
            }

            let mut stmt = self
                .file_db
                .prepare_cached(STMT_QUERY_BY_PARENT_INO)
                .unwrap();
            let items = stmt
                .query_map(
                    params![
                        FileInfoRow::from(FileInfo::with_parent_ino(file_info.ino)).parent_ino,
                        // The offset includes . and .., both which are not included in the
                        // database, so the SELECT offset must be adjusted. Since the offset could
                        // be negative, set it to 0 in that case.
                        0.max(offset - 2)
                    ],
                    |row| Ok(FileInfo::from(row)),
                )
                .unwrap();
            for (off, item) in items.enumerate() {
                let item = item.unwrap();

                // Here the item is added to the directory listing. It is important to note that
                // the offset parameter is quite crucial for correct function. The offset parameter
                // is used for succeeding calls to start with the next item after the last item
                // from the previous call. So, the offset parameter has to be one more than the
                // index of the current item. Furthermore, since "." and ".." have been added
                // manually as to not pollute the database with them, they also have to be handled
                // properly. They always get inserted in positions 0 and 1 respectively. If the
                // call starts at offset 0, then both of the directory hardlinks are included and
                // the offset must be increased by 2. If the starting offset is 1, then only "."
                // has been already added. For the additional "..", the offset has to be increased
                // by 1. If the offset is greater than 1, then the hardlinks have been taken care
                // of and the offset is already correct.
                reply.add(
                    item.ino,
                    offset
                        + if offset == 0 {
                            2
                        } else if offset == 1 {
                            1
                        } else {
                            0
                        }
                        + off as i64
                        + 1,
                    if item.vdir {
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
