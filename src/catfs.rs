use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::{fs, thread};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request,
};
use libc::ENOENT;
use rusqlite::{params, Connection};

use crate::{
    convert_filetype, convert_metadata_to_attr, Config, DropHookFn, FileHandle, FileInfo,
    FileInfoRow, Shared, CONFIG_FILE_NAME, INO_FIRST_FREE, INO_OUTSIDE, INO_ROOT, STMT_CREATE,
    STMT_CREATE_INDEX_PARENT_INO_FILE_NAME, STMT_INSERT, STMT_QUERY_BY_PARENT_INO,
};

pub(crate) struct CatFS {
    file_db: Connection,
    file_handles: HashMap<u64, Vec<FileHandle>>,
    next_fh: u64,
    config: Config,
    drop_hook: DropHookFn,
}

impl Shared for CatFS {
    fn file_db(&self) -> &Connection {
        &self.file_db
    }

    fn get_attr_from_file_info(&self, file_info: &FileInfo) -> FileAttr {
        if file_info.vdir {
            let parts = self.get_files_info_from_parent_ino(file_info.ino);
            let attrs = parts
                .iter()
                .map(|info| {
                    convert_metadata_to_attr(
                        fs::symlink_metadata(&info.path).unwrap(),
                        Some(info.ino),
                    )
                })
                .collect::<Vec<_>>();
            let mut attr = attrs.get(0).unwrap().clone();
            attr.ino = file_info.ino;
            attr.blocks = attrs.iter().map(|attr| attr.blocks).sum();
            attr.size = attrs.iter().map(|attr| attr.size).sum();
            attr
        } else {
            let attr = convert_metadata_to_attr(
                fs::symlink_metadata(&file_info.path).unwrap(),
                Some(file_info.ino),
            );
            attr
        }
    }
}

impl CatFS {
    pub(crate) fn new(mirror: &OsStr, drop_hook: DropHookFn) -> Self {
        let config = serde_json::from_str(
            &fs::read_to_string(Path::new(&mirror).join(CONFIG_FILE_NAME))
                .expect("SCFS config file not found"),
        )
        .expect("SCFS config file contains invalid JSON");

        let file_db = Connection::open_in_memory().unwrap();

        file_db.execute(STMT_CREATE, []).unwrap();

        CatFS::populate(&file_db, &mirror, INO_OUTSIDE, INO_FIRST_FREE);

        file_db
            .execute(STMT_CREATE_INDEX_PARENT_INO_FILE_NAME, [])
            .unwrap();

        {
            let query = "UPDATE Files SET vdir = 1
                 WHERE ino IN (
                    SELECT parent_ino FROM Files WHERE part != 0
                )";
            let mut stmt = file_db.prepare(query).unwrap();
            stmt.execute([]).unwrap();
        }

        let file_handles = Default::default();

        CatFS {
            file_db,
            file_handles,
            next_fh: 0,
            config,
            drop_hook,
        }
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

    fn populate<P: AsRef<Path>>(
        file_db: &Connection,
        path: P,
        parent_ino: u64,
        mut next_ino: u64,
    ) -> u64 {
        let path = path.as_ref();

        let meta = path.symlink_metadata().unwrap();

        if let None = convert_filetype(meta.file_type()) {
            return next_ino;
        }

        let attr = convert_metadata_to_attr(meta, None);

        if path.file_name().unwrap() == CONFIG_FILE_NAME {
            return next_ino;
        }

        let ino = if parent_ino == INO_OUTSIDE {
            INO_ROOT
        } else {
            let ino = next_ino;
            next_ino += 1;
            ino
        };

        let file_info = FileInfoRow::from(FileInfo {
            ino,
            parent_ino,
            path: OsString::from(path),
            file_name: path.file_name().unwrap().into(),
            part: if let FileType::RegularFile = attr.kind {
                path.file_name().unwrap().to_str().unwrap()[5..]
                    .parse::<u64>()
                    .unwrap()
                    + 1
            } else {
                0
            },
            vdir: false,
            symlink: attr.kind == FileType::Symlink,
        });

        file_db
            .prepare_cached(STMT_INSERT)
            .unwrap()
            .execute(params![
                file_info.ino,
                file_info.parent_ino,
                file_info.path,
                file_info.file_name,
                file_info.part,
                file_info.vdir,
                file_info.symlink,
            ])
            .unwrap();

        if let FileType::Directory = attr.kind {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                next_ino = CatFS::populate(&file_db, entry.path(), ino, next_ino);
            }
        }

        next_ino
    }
}

impl Drop for CatFS {
    fn drop(&mut self) {
        let _ = &(self.drop_hook)();
    }
}

impl Filesystem for CatFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        Shared::lookup(self, _req, parent, name, reply);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        Shared::getattr(self, _req, ino, reply);
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        Shared::readlink(self, _req, ino, reply);
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        let files = self.get_files_info_from_parent_ino(ino);

        let fhs = files
            .iter()
            .map(|file| FileHandle {
                file: file.path.clone(),
                start: 0,
                end: 0,
            })
            .collect();

        let fh = self.next_fh;
        self.next_fh += 1;
        self.file_handles.insert(fh, fhs);
        reply.opened(fh, 0);
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let offset = offset as usize;
        let size = size as usize;

        let file_size = self
            .get_attr_from_file_info(&self.get_file_info_from_ino(ino).unwrap())
            .size as usize;

        let offset = offset.min(file_size);
        let size = size.min(file_size - offset);

        if size == 0 {
            reply.data(&[]);
            return;
        }

        let part_start = offset / self.config.blocksize as usize;
        let part_end = (offset + size - 1) / self.config.blocksize as usize;

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

        let blocksize = self.config.blocksize;

        thread::spawn(move || {
            let part_start = 0;

            let bytes = files
                .iter()
                .enumerate()
                .map(|(part, file)| {
                    let mut file = BufReader::new(File::open(file).unwrap());

                    file.seek(SeekFrom::Start(if part == part_start {
                        offset as u64 % blocksize
                    } else {
                        0
                    }))
                    .unwrap();

                    file.bytes().map(|b| b.unwrap())
                })
                .flatten()
                .take(size)
                .collect::<Vec<_>>();

            reply.data(&bytes);
        });
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
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
                    if reply.add(file_info.ino, 1, FileType::Directory, ".") {
                        unreachable!()
                    }
                }
                if reply.add(
                    if file_info.parent_ino == INO_OUTSIDE {
                        file_info.ino
                    } else {
                        file_info.parent_ino
                    },
                    2,
                    FileType::Directory,
                    "..",
                ) {
                    unreachable!()
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
                let is_full = reply.add(
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
                    if item.symlink {
                        FileType::Symlink
                    } else if item.vdir {
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

#[cfg(test)]
mod tests {
    use std::fs::DirEntry;
    use std::iter;
    use std::ops::Deref;

    use fuser::BackgroundSession;
    use rand::{thread_rng, Rng, RngCore};
    use tempfile::{tempdir, TempDir};

    use crate::mount;
    use crate::shared::tests::{check_symlinks, create_files_and_symlinks};

    use super::*;

    // Helper struct to keep necessary variables in scope. To not make the compiler complain,
    // prefix them with an underscore. If for example the TempDir variables are not kept in scope
    // this way, the directories would be deleted before the tests can be run.
    struct TempSession {
        _session: BackgroundSession,
        _mirror: TempDir,
        pub(crate) mountpoint: TempDir,
    }

    fn mount_and_create_files_with_symlinks<'a>(
        files: &Vec<(String, Vec<u8>)>,
        symlinks: Vec<(String, String)>,
    ) -> Result<TempSession, std::io::Error> {
        let mirror = tempdir()?;
        let mountpoint = tempdir()?;

        create_files_and_symlinks(mirror.path(), &files, &symlinks)?;

        let fs = CatFS::new(mirror.path().as_os_str(), Box::new(|| ()));

        let session = mount(fs, &mountpoint, Vec::new());

        Ok(TempSession {
            _mirror: mirror,
            mountpoint,
            _session: session,
        })
    }

    fn mount_and_create_files<'a>(
        files: &Vec<(String, Vec<u8>)>,
    ) -> Result<TempSession, std::io::Error> {
        mount_and_create_files_with_symlinks(files, Vec::new())
    }

    fn create_random_file_tuples(
        blocksize: usize,
        num_files: usize,
        max_num_fragments: usize,
    ) -> Vec<(String, Vec<u8>)> {
        let mut rng = thread_rng();

        (0..num_files)
            .flat_map(|file_num| {
                let max_num_fragments = rng.gen_range(1..max_num_fragments);
                (0..max_num_fragments)
                    .map(|fragment_num| {
                        let file_name = format!("file_{}/scfs.{:010}", file_num, fragment_num);

                        let fragment_size = if fragment_num == max_num_fragments - 1 && rng.gen() {
                            rng.gen_range(1..(blocksize + 1))
                        } else {
                            blocksize
                        };

                        let mut content = vec![0u8; fragment_size];
                        rng.fill_bytes(&mut content);

                        (file_name, content)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    }

    fn create_config_file_tuple(config: Option<Config>) -> (String, Vec<u8>) {
        (
            CONFIG_FILE_NAME.to_string(),
            serde_json::to_vec(&config.unwrap_or_default()).unwrap(),
        )
    }

    fn with_config_file(files: Vec<(String, Vec<u8>)>, config: Config) -> Vec<(String, Vec<u8>)> {
        files
            .into_iter()
            .chain(iter::once(create_config_file_tuple(Some(config))))
            .collect()
    }

    fn check_files(
        mountpoint: &Path,
        files_expected: Vec<(String, Vec<u8>)>,
    ) -> Result<(), std::io::Error> {
        let files_actual = fs::read_dir(mountpoint)?
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();

        let mut files_expected_map = HashMap::new();
        for (file_name, content_chunk) in files_expected {
            if file_name == CONFIG_FILE_NAME {
                continue;
            }

            let file_name = mountpoint.join(file_name).parent().unwrap().to_path_buf();
            let content = files_expected_map.entry(file_name).or_insert(Vec::new());
            for c in content_chunk {
                content.push(c)
            }
        }

        assert_eq!(files_actual.len(), files_expected_map.len());

        for file in files_actual {
            let content_actual = fs::read(&file).unwrap();
            let content_actual = content_actual.deref();

            let content_expected = files_expected_map.get(&file).unwrap();
            let content_expected = content_expected.deref();

            assert_eq!(content_actual, content_expected)
        }

        Ok(())
    }

    #[test]
    #[should_panic(expected = "SCFS config file not found")]
    fn test_empty_mirror() {
        // Since a valid SplitFS needs a config file, panic if there is no such file

        mount_and_create_files(&Vec::new()).unwrap();
    }

    #[test]
    fn test_empty_mirror_with_config() -> Result<(), std::io::Error> {
        // If there is a valid config file, but nothing else, then the CatFS mountpoint is
        // completely empty.

        let files = vec![create_config_file_tuple(None)];

        let session = mount_and_create_files(&files)?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 0);

        Ok(())
    }

    #[test]
    #[should_panic(expected = "SCFS config file contains invalid JSON")]
    fn test_empty_mirror_with_wrong_config() {
        // An invalid config file must result in a panic

        let files = vec![(CONFIG_FILE_NAME.to_string(), "{}".into())];

        mount_and_create_files(&files).unwrap();
    }

    #[test]
    fn test_blocksize_1() -> Result<(), std::io::Error> {
        let config = Config::default().blocksize(1);
        let blocksize = config.blocksize as usize;
        let num_files = 50;
        let max_num_fragments = 100;

        let files_expected = with_config_file(
            create_random_file_tuples(blocksize, num_files, max_num_fragments),
            config,
        );

        let session = mount_and_create_files(&files_expected)?;

        check_files(session.mountpoint.path(), files_expected)
    }

    #[test]
    fn test_blocksize_1kb() -> Result<(), std::io::Error> {
        let config = Config::default().blocksize(1024);
        let blocksize = config.blocksize as usize;
        let num_files = 20;
        let max_num_fragments = 10;

        let files_expected = with_config_file(
            create_random_file_tuples(blocksize, num_files, max_num_fragments),
            config,
        );

        let session = mount_and_create_files(&files_expected)?;

        check_files(session.mountpoint.path(), files_expected)
    }

    #[test]
    #[ignore]
    fn test_expensive_blocksize_default() -> Result<(), std::io::Error> {
        let config = Config::default();
        let blocksize = config.blocksize as usize;
        let num_files = 10;
        let max_num_fragments = 5;

        let files_expected = with_config_file(
            create_random_file_tuples(blocksize, num_files, max_num_fragments),
            config,
        );

        let session = mount_and_create_files(&files_expected)?;

        check_files(session.mountpoint.path(), files_expected)
    }

    #[test]
    fn test_symlink_relative_file() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such. A
        // relative symlink pointing to a file inside the chunked directory should translate into
        // the same concatenated file as the real file.

        let config = Config::default().blocksize(1);
        let blocksize = config.blocksize as usize;
        let num_files = 1;
        let max_num_fragments = 5;

        let files = with_config_file(
            create_random_file_tuples(blocksize, num_files, max_num_fragments),
            config,
        );

        let mut symlink_map: HashMap<String, String> = HashMap::new();
        symlink_map.insert(
            "link_rel".into(),
            files.first().unwrap().0.split("/").next().unwrap().into(),
        );

        let session = mount_and_create_files_with_symlinks(
            &files,
            symlink_map.clone().into_iter().collect(),
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), num_files + symlink_map.len());

        let symlinks_found: Vec<&DirEntry> = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        let contents = entries
            .iter()
            .map(|file| fs::read(file.path()).unwrap())
            .collect::<Vec<_>>();

        assert!(contents
            .iter()
            .all(|content| content.eq(contents.first().unwrap())));

        check_symlinks(&mut symlink_map, &symlinks_found)
    }

    #[test]
    fn test_symlink_absolute_dir() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such.

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_abs".into(), "/".into());

        let session = mount_and_create_files_with_symlinks(
            &with_config_file(Vec::new(), Config::default()),
            symlink_map.clone().into_iter().collect(),
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), symlink_map.len());

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        assert!(symlinks_found.first().unwrap().path().is_dir());

        check_symlinks(&mut symlink_map, &symlinks_found)
    }

    #[test]
    fn test_symlink_broken() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such. If
        // the target does not exist, just show a broken symlink, do not panic out.

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_rel".into(), "a/b/c".into());
        symlink_map.insert("link_abs".into(), "/home/nobody/nothing".into());

        let session = mount_and_create_files_with_symlinks(
            &with_config_file(Vec::new(), Config::default()),
            symlink_map.clone().into_iter().collect(),
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), symlink_map.len());

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        check_symlinks(&mut symlink_map, &symlinks_found)
    }
}
