use std::ffi::{OsStr, OsString};
use std::fs;

use fuse::{FileAttr, ReplyAttr, ReplyData, ReplyEntry, Request};
use libc::ENOENT;
use rusqlite::{params, Connection, Error};

use crate::{FileInfo, FileInfoRow, STMT_QUERY_BY_INO, STMT_QUERY_BY_PARENT_INO_AND_FILENAME, TTL};

pub trait Shared {
    fn file_db(&self) -> &Connection;

    fn get_file_info_from_ino(&self, ino: u64) -> Result<FileInfo, Error> {
        let ino = FileInfoRow::from(FileInfo::with_ino(ino)).ino;

        let mut stmt = self.file_db().prepare_cached(STMT_QUERY_BY_INO).unwrap();

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
            .file_db()
            .prepare_cached(STMT_QUERY_BY_PARENT_INO_AND_FILENAME)
            .unwrap();

        let file_name = FileInfo::default()
            .file_name(file_name)
            .into_file_info_row()
            .file_name;

        let file_info = stmt
            .query_map(params![parent_ino, file_name], |row| {
                Ok(FileInfo::from(row))
            })
            .unwrap()
            .next();

        match file_info {
            Some(f) => Ok(f.unwrap()),
            None => Err(Error::QueryReturnedNoRows),
        }
    }

    fn get_attr_from_file_info(&self, file_info: &FileInfo) -> FileAttr;

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

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        let path = self.get_file_info_from_ino(ino).unwrap().path;
        let target = fs::read_link(path).unwrap();
        let target = target.to_str().unwrap().as_bytes();
        reply.data(target);
    }
}
