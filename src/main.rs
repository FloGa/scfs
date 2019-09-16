use std::env;
use std::ffi::OsStr;

use scfs::SplitFS;

fn main() {
    let mirror = env::args_os().nth(1).unwrap();
    let mountpoint = env::args_os().nth(2).unwrap();
    let fs = SplitFS::new(mirror);
    let options = ["-o", "ro", "-o", "fsname=scfs"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();
    fuse::mount(fs, &mountpoint, &options).unwrap();
}
