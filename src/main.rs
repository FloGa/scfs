use std::env;
use std::ffi::OsStr;
use std::sync::mpsc::channel;

use scfs::SplitFS;

fn main() {
    let mirror = env::args_os().nth(1).unwrap();
    let mountpoint = env::args_os().nth(2).unwrap();
    let fs = SplitFS::new(mirror);
    let options = ["-o", "ro", "-o", "fsname=scfs"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();

    let (tx_quitter, rx_quitter) = channel();

    ctrlc::set_handler(move || {
        tx_quitter.send(true).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    let _session = unsafe { fuse::spawn_mount(fs, &mountpoint, &options).unwrap() };

    rx_quitter.recv().expect("Could not join quitter channel.");
}
