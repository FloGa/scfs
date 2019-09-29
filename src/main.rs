use std::env;
use std::ffi::OsStr;
use std::sync::mpsc::channel;

use scfs::{CatFS, SplitFS};

fn main() {
    let mode = env::args_os().nth(1).unwrap();
    let mirror = env::args_os().nth(2).unwrap();
    let mountpoint = env::args_os().nth(3).unwrap();

    let mode = mode.into_string().unwrap();

    if !mode.starts_with("--mode=") {
        panic!("Mode flag must be given as --mode=cat or --mode=split");
    };

    let mode = &mode[7..];

    let options = ["-o", "ro", "-o", "fsname=scfs"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();

    let (tx_quitter, rx_quitter) = channel();

    ctrlc::set_handler(move || {
        tx_quitter.send(true).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    let _session = {
        if mode == "cat" {
            let fs = CatFS::new(mirror);
            unsafe { fuse::spawn_mount(fs, &mountpoint, &options).unwrap() }
        } else if mode == "split" {
            let fs = SplitFS::new(mirror);
            unsafe { fuse::spawn_mount(fs, &mountpoint, &options).unwrap() }
        } else {
            panic!("Unknown mode: {:?}", mode);
        }
    };

    rx_quitter.recv().expect("Could not join quitter channel.");
}
