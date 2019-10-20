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
    CONFIG_FILE_NAME, INO_CONFIG, INO_OUTSIDE, INO_ROOT, STMT_CREATE, STMT_INSERT,
    STMT_QUERY_BY_INO, STMT_QUERY_BY_PARENT_INO, TTL,
};

pub struct SplitFS {
    file_db: Connection,
    file_handles: HashMap<u64, FileHandle>,
    config: Config,
    config_json: String,
}

impl SplitFS {
    pub fn new(mirror: &OsStr) -> Self {
        let file_db = Connection::open_in_memory().unwrap();

        file_db.execute(STMT_CREATE, NO_PARAMS).unwrap();

        SplitFS::populate(&file_db, &mirror, INO_OUTSIDE);

        let file_handles = Default::default();

        let config = Config {};
        let config_json = serde_json::to_string(&config).unwrap();

        SplitFS {
            file_db,
            file_handles,
            config,
            config_json,
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

    fn get_attr_from_file_info(&self, file_info: &FileInfo) -> FileAttr {
        if file_info.part == 0 {
            let mut attr = convert_metadata_to_attr(
                fs::metadata(&file_info.path).unwrap(),
                Some(file_info.ino),
            );
            attr.kind = FileType::Directory;
            attr.blocks = 0;
            attr.perm = 0o755;
            attr
        } else {
            let mut attr = convert_metadata_to_attr(
                fs::metadata(
                    self.get_file_info_from_ino(file_info.parent_ino)
                        .unwrap()
                        .path,
                )
                .unwrap(),
                Some(file_info.ino),
            );
            attr.size = u64::min(BLOCK_SIZE, attr.size - (file_info.part - 1) * BLOCK_SIZE);
            attr
        }
    }

    fn get_config_attr(&self) -> FileAttr {
        let file_info = self.get_file_info_from_ino(INO_ROOT).unwrap();
        let mut attr = self.get_attr_from_file_info(&file_info);
        attr.ino = INO_CONFIG;
        attr.size = self.config_json.len() as u64;
        attr.blocks = 1;
        attr.kind = FileType::RegularFile;
        attr
    }

    fn populate<P: AsRef<Path>>(file_db: &Connection, path: P, parent_ino: u64) {
        let path = path.as_ref();

        let mut attr = convert_metadata_to_attr(path.metadata().unwrap(), None);

        attr.ino = if parent_ino == INO_OUTSIDE {
            INO_ROOT
        } else {
            time::precise_time_ns()
        };

        let file_info = FileInfoRow::from(FileInfo {
            ino: attr.ino,
            parent_ino,
            path: OsString::from(path),
            part: 0,
            vdir: attr.kind == FileType::RegularFile,
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

        if let FileType::RegularFile = attr.kind {
            let blocks = f64::ceil(attr.size as f64 / BLOCK_SIZE as f64) as u64;
            for i in 0..blocks {
                let file_info = FileInfoRow::from(FileInfo {
                    ino: attr.ino + i + 1,
                    parent_ino: attr.ino,
                    path: OsString::from(path.join(format!("scfs.{:010}", i))),
                    part: i + 1,
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
            }
        }

        if path.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                SplitFS::populate(&file_db, entry.path(), attr.ino);
            }
        }
    }
}

impl Filesystem for SplitFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == INO_ROOT && name == CONFIG_FILE_NAME {
            let attr = self.get_config_attr();
            reply.entry(&TTL, &attr, 0);
            return;
        }

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
        if ino == INO_CONFIG {
            let attr = self.get_config_attr();
            reply.attr(&TTL, &attr);
            return;
        }

        let file_info = self.get_file_info_from_ino(ino);
        if let Ok(file_info) = file_info {
            let attr = self.get_attr_from_file_info(&file_info);
            reply.attr(&TTL, &attr)
        } else {
            reply.error(ENOENT)
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        if ino == INO_CONFIG {
            reply.opened(0, 0);
            return;
        }

        let file_info = self.get_file_info_from_ino(ino);
        if let Ok(file_info) = file_info {
            let file = self
                .get_file_info_from_ino(file_info.parent_ino)
                .unwrap()
                .path;

            let start = (file_info.part - 1) * BLOCK_SIZE;
            let end = start + BLOCK_SIZE;
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
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        if ino == INO_CONFIG {
            reply.data(self.config_json.as_ref());
            return;
        }

        let offset = offset as u64;
        let size = size as u64;

        let handle = self.file_handles.get(&fh).unwrap();
        let file = handle.file.clone();

        let offset = offset.min(handle.end - handle.start);
        let size = size.min(handle.end - handle.start - offset);
        let start = handle.start;

        thread::spawn(move || {
            let mut file = BufReader::new(File::open(file).unwrap());

            file.seek(SeekFrom::Start(start + offset)).unwrap();

            let bytes = file
                .take(size)
                .bytes()
                .map(|b| b.unwrap())
                .collect::<Vec<_>>();

            reply.data(&bytes);
        });
    }

    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        if ino == INO_CONFIG {
            reply.ok();
            return;
        }

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
            // . and .. make 2 and optionally 1 for .scfs_config
            let additional_offset_max = 2 + if file_info.ino == 1 { 1 } else { 0 };

            let mut additional_offset = 0;
            if offset < 3 {
                if offset < 1 {
                    reply.add(file_info.ino, 1, FileType::Directory, ".");
                    additional_offset += 1;
                }

                if offset < 2 {
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
                    additional_offset += 1;
                }

                if offset < 3 {
                    if file_info.ino == INO_ROOT {
                        reply.add(INO_CONFIG, 3, FileType::RegularFile, CONFIG_FILE_NAME);
                        additional_offset += 1;
                    }
                }
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
                        0.max(offset - additional_offset_max)
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
                let is_full = reply.add(
                    item.ino,
                    offset + additional_offset + off as i64 + 1,
                    if item.part > 0 {
                        FileType::RegularFile
                    } else {
                        FileType::Directory
                    },
                    Path::new(&item.path).file_name().unwrap(),
                );

                if is_full {
                    break;
                }
            }
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }
}
