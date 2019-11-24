use std::ffi::OsStr;
use std::sync::mpsc::channel;

use clap::{crate_authors, crate_description, crate_name, crate_version, value_t, App, Arg};

use scfs::{CatFS, Config, SplitFS};

const ARG_MODE: &str = "mode";
const ARG_MIRROR: &str = "mirror";
const ARG_MOUNTPOINT: &str = "mountpoint";
const ARG_BLOCKSIZE: &str = "blocksize";

fn main() {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::with_name(ARG_MODE)
                .short(&ARG_MODE[0..1])
                .long(ARG_MODE)
                .value_name(ARG_MODE.to_uppercase().as_str())
                .help("Sets the desired mode")
                .takes_value(true)
                .possible_values(&["split", "cat"])
                .required(true),
        )
        .arg(
            Arg::with_name(ARG_BLOCKSIZE)
                .short(&ARG_BLOCKSIZE[0..1])
                .long(ARG_BLOCKSIZE)
                .value_name(ARG_BLOCKSIZE.to_uppercase().as_str())
                .help("Sets the desired blocksize")
                .takes_value(true)
                .default_value("2097152"),
        )
        .arg(
            Arg::with_name(ARG_MIRROR)
                .help("Defines the directory that will be mirrored")
                .required(true),
        )
        .arg(
            Arg::with_name(ARG_MOUNTPOINT)
                .help("Defines the mountpoint, where the mirror will be accessible")
                .required(true),
        )
        .get_matches();

    let mode = matches.value_of(ARG_MODE).unwrap();
    let mirror = matches.value_of_os(ARG_MIRROR).unwrap();
    let mountpoint = matches.value_of_os(ARG_MOUNTPOINT).unwrap();

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
            let blocksize = value_t!(matches, ARG_BLOCKSIZE, u64).unwrap_or_else(|e| e.exit());
            let config = Config::default().blocksize(blocksize);
            let fs = SplitFS::new(mirror, config);
            unsafe { fuse::spawn_mount(fs, &mountpoint, &options).unwrap() }
        } else {
            unreachable!()
        }
    };

    rx_quitter.recv().expect("Could not join quitter channel.");
}
