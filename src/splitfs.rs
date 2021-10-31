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
use rusqlite::{params, Connection, NO_PARAMS};

use crate::{
    convert_filetype, convert_metadata_to_attr, Config, DropHookFn, FileHandle, FileInfo,
    FileInfoRow, Shared, CONFIG_FILE_NAME, INO_CONFIG, INO_OUTSIDE, INO_ROOT, STMT_CREATE,
    STMT_CREATE_INDEX_PARENT_INO_FILE_NAME, STMT_INSERT, STMT_QUERY_BY_PARENT_INO, TTL,
};

pub(crate) struct SplitFS {
    file_db: Connection,
    file_handles: HashMap<u64, FileHandle>,
    config: Config,
    config_json: String,
    drop_hook: DropHookFn,
}

impl Shared for SplitFS {
    fn file_db(&self) -> &Connection {
        &self.file_db
    }

    fn get_attr_from_file_info(&self, file_info: &FileInfo) -> FileAttr {
        if file_info.symlink {
            let attr = convert_metadata_to_attr(
                fs::symlink_metadata(&file_info.path).unwrap(),
                Some(file_info.ino),
            );
            attr
        } else if file_info.part == 0 {
            let mut attr = convert_metadata_to_attr(
                fs::symlink_metadata(&file_info.path).unwrap(),
                Some(file_info.ino),
            );
            attr.kind = FileType::Directory;
            attr.blocks = 0;
            attr.perm = 0o755;
            attr
        } else {
            let mut attr = convert_metadata_to_attr(
                fs::symlink_metadata(
                    self.get_file_info_from_ino(file_info.parent_ino)
                        .unwrap()
                        .path,
                )
                .unwrap(),
                Some(file_info.ino),
            );
            attr.size = u64::min(
                self.config.blocksize,
                attr.size - (file_info.part - 1) * self.config.blocksize,
            );
            attr
        }
    }
}

impl SplitFS {
    pub(crate) fn new(mirror: &OsStr, config: Config, drop_hook: DropHookFn) -> Self {
        let file_db = Connection::open_in_memory().unwrap();

        file_db.execute(STMT_CREATE, NO_PARAMS).unwrap();

        SplitFS::populate(&file_db, &mirror, &config, INO_OUTSIDE);

        file_db
            .execute(STMT_CREATE_INDEX_PARENT_INO_FILE_NAME, NO_PARAMS)
            .unwrap();

        let file_handles = Default::default();

        let config_json = serde_json::to_string(&config).unwrap();

        SplitFS {
            file_db,
            file_handles,
            config,
            config_json,
            drop_hook,
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

    fn populate<P: AsRef<Path>>(file_db: &Connection, path: P, config: &Config, parent_ino: u64) {
        let path = path.as_ref();

        let meta = path.symlink_metadata().unwrap();

        if let None = convert_filetype(meta.file_type()) {
            return;
        }

        let mut attr = convert_metadata_to_attr(meta, None);

        attr.ino = if parent_ino == INO_OUTSIDE {
            INO_ROOT
        } else {
            time::precise_time_ns()
        };

        let file_info = FileInfoRow::from(FileInfo {
            ino: attr.ino,
            parent_ino,
            path: OsString::from(path),
            file_name: path.file_name().unwrap().into(),
            part: 0,
            vdir: attr.kind == FileType::RegularFile,
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

        match attr.kind {
            FileType::RegularFile => {
                // Create at least one chunk, even if it is empty. This way, we can differentiate
                // between an empty file and an empty directory.
                let blocks = 1.max(f64::ceil(attr.size as f64 / config.blocksize as f64) as u64);
                for i in 0..blocks {
                    let file_name = format!("scfs.{:010}", i).into();
                    let file_info = FileInfoRow::from(FileInfo {
                        ino: attr.ino + i + 1,
                        parent_ino: attr.ino,
                        path: OsString::from(path.join(&file_name)),
                        file_name,
                        part: i + 1,
                        vdir: false,
                        symlink: false,
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
                }
            }

            FileType::Directory => {
                for entry in fs::read_dir(path).unwrap() {
                    let entry = entry.unwrap();
                    SplitFS::populate(&file_db, entry.path(), &config, attr.ino);
                }
            }

            _ => {}
        }
    }
}

impl Drop for SplitFS {
    fn drop(&mut self) {
        let _ = &(self.drop_hook)();
    }
}

impl Filesystem for SplitFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == INO_ROOT && name == CONFIG_FILE_NAME {
            let attr = self.get_config_attr();
            reply.entry(&TTL, &attr, 0);
            return;
        }

        Shared::lookup(self, _req, parent, name, reply);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if ino == INO_CONFIG {
            let attr = self.get_config_attr();
            reply.attr(&TTL, &attr);
            return;
        }

        Shared::getattr(self, _req, ino, reply);
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        Shared::readlink(self, _req, ino, reply);
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
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

            let start = (file_info.part - 1) * self.config.blocksize;
            let end = start + self.config.blocksize;
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
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
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
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
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
                    if reply.add(file_info.ino, 1, FileType::Directory, ".") {
                        unreachable!()
                    }
                    additional_offset += 1;
                }

                if offset < 2 {
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
                    additional_offset += 1;
                }

                if offset < 3 {
                    if file_info.ino == INO_ROOT {
                        if reply.add(INO_CONFIG, 3, FileType::RegularFile, CONFIG_FILE_NAME) {
                            unreachable!()
                        }
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
                    if item.symlink {
                        FileType::Symlink
                    } else if item.part > 0 {
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
    use std::fs::{read, DirEntry};
    use std::path::PathBuf;

    use fuser::BackgroundSession;
    use rand::{Rng, RngCore};
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
        files: Vec<(String, Vec<u8>)>,
        symlinks: Vec<(String, String)>,
        config: Option<Config>,
    ) -> Result<TempSession, std::io::Error> {
        let mirror = tempdir()?;
        let mountpoint = tempdir()?;

        create_files_and_symlinks(mirror.path(), &files, &symlinks)?;

        let fs = SplitFS::new(
            mirror.path().as_os_str(),
            config.unwrap_or_default(),
            Box::new(|| ()),
        );

        let session = mount(fs, &mountpoint, Vec::new());

        Ok(TempSession {
            _mirror: mirror,
            mountpoint,
            _session: session,
        })
    }

    fn mount_and_create_files<'a>(
        files: Vec<(String, Vec<u8>)>,
        config: Option<Config>,
    ) -> Result<TempSession, std::io::Error> {
        mount_and_create_files_with_symlinks(files, Vec::new(), config)
    }

    fn mount_and_create_seq_files<'a>(
        num_files: usize,
        config: Option<Config>,
    ) -> Result<TempSession, std::io::Error> {
        let files = (0..num_files)
            .map(|i| {
                let i_as_string = format!("{}", i);
                (i_as_string.clone(), i_as_string.into_bytes())
            })
            .collect::<Vec<_>>();

        mount_and_create_files(files, config)
    }

    fn list_files_in_path(path: PathBuf) -> Vec<PathBuf> {
        fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>()
    }

    fn list_files_in_directories(dirs: &Vec<&DirEntry>) -> Vec<(String, Vec<PathBuf>)> {
        dirs.iter()
            .map(|item| {
                let file_name = item.file_name().into_string().unwrap();
                let files = list_files_in_path(item.path());
                (file_name, files)
            })
            .collect::<Vec<_>>()
    }

    fn check_files(num_files: usize, dirs: &Vec<&DirEntry>, files: &Vec<(String, Vec<PathBuf>)>) {
        assert_eq!(dirs.len(), num_files);

        assert_eq!(
            files.iter().filter(|&(_, files)| files.len() == 1).count(),
            num_files
        );

        assert_eq!(
            files
                .iter()
                .filter(|&(file_name, files)| {
                    let contents = files
                        .iter()
                        .flat_map(|entry| fs::read(entry).unwrap())
                        .collect::<Vec<_>>();
                    file_name.clone().into_bytes() == contents
                })
                .count(),
            num_files
        );
    }

    #[test]
    fn test_empty_mirror() -> Result<(), std::io::Error> {
        // Even with an empty mirror, there will be at least one file, namely the virtual config
        // file, with a default Config struct as content

        let session = mount_and_create_seq_files(0, None)?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1);

        let file = entries.get(0).unwrap();

        assert_eq!(file.file_name(), CONFIG_FILE_NAME);

        assert_eq!(
            fs::read_to_string(file.path())?,
            serde_json::to_string(&Config::default())?
        );

        Ok(())
    }

    #[test]
    fn test_empty_mirror_custom_config() -> Result<(), std::io::Error> {
        // Even with an empty mirror, there will be at least one file, namely the virtual config
        // file, with the custom Config struct as content

        let config = Config::default().blocksize(1);

        let session = mount_and_create_seq_files(0, Some(config))?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1);

        let file = entries.get(0).unwrap();

        assert_eq!(file.file_name(), CONFIG_FILE_NAME);

        assert_eq!(
            fs::read_to_string(file.path())?,
            serde_json::to_string(&config)?
        );

        Ok(())
    }

    #[test]
    fn test_empty_file() -> Result<(), std::io::Error> {
        let num_files = 1;

        let files = vec![("empty_file".to_string(), vec![])];

        let session = mount_and_create_files(files, None)?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), num_files + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let dirs = entries
            .iter()
            .filter(|item| item.path().is_dir())
            .collect::<Vec<_>>();

        assert_eq!(dirs.len(), num_files);

        let files = list_files_in_directories(&dirs);

        assert_eq!(
            files.iter().filter(|&(_, files)| files.len() == 1).count(),
            num_files
        );

        assert_eq!(
            files
                .iter()
                .filter(|&(_, files)| {
                    let contents = files
                        .iter()
                        .flat_map(|entry| fs::read(entry).unwrap())
                        .collect::<Vec<_>>();
                    contents.is_empty()
                })
                .count(),
            num_files
        );

        Ok(())
    }

    #[test]
    fn test_100_small_files() -> Result<(), std::io::Error> {
        // With an mirror containing 100 small files, there should be 101 entries in the
        // mountpoint: The config file and 100 folders representing the files. Each folder should
        // contain a single part with the file contents, which are equal to the file name.

        let num_files = 100;

        let session = mount_and_create_seq_files(num_files, None)?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), num_files + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let dirs = entries
            .iter()
            .filter(|item| item.path().is_dir())
            .collect::<Vec<_>>();

        let files = list_files_in_directories(&dirs);

        check_files(num_files, &dirs, &files);

        Ok(())
    }

    #[test]
    fn test_100_small_files_in_subfolders() -> Result<(), std::io::Error> {
        // Same as above, but with multiple nested folders.

        let num_files = 100;

        let mut rng = rand::thread_rng();
        let files = (0..num_files)
            .map(|i| {
                let file_name = format!(
                    "{}/{}/{}/{}/{}/{}",
                    i,
                    rng.gen::<u32>(),
                    rng.gen::<u32>(),
                    rng.gen::<u32>(),
                    rng.gen::<u32>(),
                    rng.gen::<u32>()
                );
                let content = format!("{}", i,);
                (file_name, content.into_bytes())
            })
            .collect::<Vec<_>>();

        let session = mount_and_create_files(files, None)?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), num_files + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let dirs = entries
            .iter()
            .filter(|item| item.path().is_dir())
            .collect::<Vec<_>>();

        let files = dirs
            .iter()
            .map(|item| {
                let file_name = item.file_name().into_string().unwrap();
                let files = fs::read_dir(item.path())
                    .unwrap()
                    .map(|entry| {
                        let mut path = entry.unwrap().path();
                        while path.is_dir() {
                            path = path.read_dir().unwrap().next().unwrap().unwrap().path();
                        }
                        path.to_path_buf()
                    })
                    .collect::<Vec<_>>();
                (file_name, files)
            })
            .collect::<Vec<_>>();

        check_files(num_files, &dirs, &files);

        Ok(())
    }

    #[test]
    fn test_big_file_bytewise() -> Result<(), std::io::Error> {
        // A big file, with a block size of 1 byte, should be splitted in as many parts as bytes.
        // By concatenating all parts together, the original content should be created.

        let config = Config::default().blocksize(1);

        let mut data = [0u8; 100];
        rand::thread_rng().fill_bytes(&mut data);
        let data = data.to_vec();

        let files = vec![("huge_file".to_string(), data.clone())];

        let session = mount_and_create_files(files, Some(config))?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1 + 1);

        let dirs = entries
            .iter()
            .filter(|item| item.path().is_dir())
            .collect::<Vec<_>>();

        assert_eq!(dirs.len(), 1);

        let dir = dirs.get(0).unwrap();

        let files = list_files_in_path(dir.path());

        assert_eq!(files.len(), data.len());

        assert_eq!(
            files
                .iter()
                .flat_map(|file| fs::read(file).unwrap())
                .collect::<Vec<_>>(),
            data
        );

        Ok(())
    }

    #[test]
    fn test_big_file_blockwise() -> Result<(), std::io::Error> {
        // A big file, with a custom block size, should be splitted in as many parts as needed so
        // that the parts are no larger than the block size. By concatenating all parts together,
        // the original content should be created.

        let blocksize = 37;

        let config = Config::default().blocksize(blocksize);

        let mut data = [0u8; 100];
        rand::thread_rng().fill_bytes(&mut data);
        let data = data.to_vec();

        let files = vec![("huge_file".to_string(), data.clone())];

        let session = mount_and_create_files(files, Some(config))?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1 + 1);

        let dirs = entries
            .iter()
            .filter(|item| item.path().is_dir())
            .collect::<Vec<_>>();

        assert_eq!(dirs.len(), 1);

        let dir = dirs.get(0).unwrap();

        let files = list_files_in_path(dir.path());

        assert_eq!(
            files.len(),
            (data.len() as f32 / blocksize as f32).ceil() as usize
        );

        assert_eq!(
            files
                .iter()
                .filter(|item| File::open(item).unwrap().bytes().count() == blocksize as usize)
                .count(),
            files.len() - 1
        );

        assert_eq!(
            File::open(files.last().unwrap()).unwrap().bytes().count(),
            data.len() % blocksize as usize
        );

        assert_eq!(
            files
                .iter()
                .flat_map(|file| fs::read(file).unwrap())
                .collect::<Vec<_>>(),
            data
        );

        Ok(())
    }

    #[test]
    fn test_symlink_relative_file() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such, no splitting or any other modification.

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_rel".into(), CONFIG_FILE_NAME.into());

        let session = mount_and_create_files_with_symlinks(
            Vec::new(),
            symlink_map.clone().into_iter().collect(),
            None,
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), symlink_map.len() + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        assert_eq!(
            read(symlinks_found.first().unwrap().path())?,
            serde_json::to_string(&Config::default())
                .unwrap()
                .into_bytes()
        );

        check_symlinks(&mut symlink_map, &symlinks_found)
    }

    #[test]
    fn test_symlink_relative_vdir() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such, no splitting or any other modification. A
        // relative symlink pointing to a file inside the chunked directory should translate into
        // the same chunked virtual directory as the real file's counterpart.

        let files = vec![(String::from("a/b/c"), String::from("42").into_bytes())];

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_rel".into(), "a/b/c".into());

        let session = mount_and_create_files_with_symlinks(
            files.clone(),
            symlink_map.clone().into_iter().collect(),
            None,
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), files.len() + symlink_map.len() + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        let parts = symlinks_found
            .first()
            .unwrap()
            .path()
            .read_dir()
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(parts.len(), 1);

        assert_eq!(
            read(parts.first().unwrap().as_ref().unwrap().path())?,
            files.first().unwrap().1
        );

        check_symlinks(&mut symlink_map, &symlinks_found)
    }

    #[test]
    fn test_symlink_absolute_dir() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such, no splitting or any other modification.

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_abs".into(), "/".into());

        let session = mount_and_create_files_with_symlinks(
            Vec::new(),
            symlink_map.clone().into_iter().collect(),
            None,
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), symlink_map.len() + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        assert!(symlinks_found.first().unwrap().path().is_dir());

        check_symlinks(&mut symlink_map, &symlinks_found)
    }

    #[test]
    fn test_symlink_broken() -> Result<(), std::io::Error> {
        // A symlink should just be presented as such, no splitting or any other modification. If
        // the target does not exist, just show a broken symlink, do not panic out.

        let mut symlink_map = HashMap::new();
        symlink_map.insert("link_rel".into(), "a/b/c".into());
        symlink_map.insert("link_abs".into(), "/home/nobody/nothing".into());

        let session = mount_and_create_files_with_symlinks(
            Vec::new(),
            symlink_map.clone().into_iter().collect(),
            None,
        )?;

        let entries = fs::read_dir(&session.mountpoint)?
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), symlink_map.len() + 1);

        assert_eq!(
            entries
                .iter()
                .filter(|item| item.file_name() == CONFIG_FILE_NAME)
                .count(),
            1
        );

        let symlinks_found = entries
            .iter()
            .filter(|item| item.file_type().unwrap().is_symlink())
            .collect::<Vec<_>>();

        check_symlinks(&mut symlink_map, &symlinks_found)
    }
}
