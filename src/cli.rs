use std::ffi::OsStr;
use std::path::Path;
use std::sync::mpsc::channel;

use clap::{
    arg_enum, crate_authors, crate_description, crate_name, crate_version, value_t, App, Arg,
    ArgMatches, Result,
};

use crate::{mount, CatFS, Config, SplitFS};

const ARG_MODE: &str = "mode";
const ARG_MIRROR: &str = "mirror";
const ARG_MOUNTPOINT: &str = "mountpoint";
const ARG_BLOCKSIZE: &str = "blocksize";
const ARG_FUSE_OPTIONS: &str = "fuse_options";
const ARG_FUSE_OPTIONS_EXTRA: &str = "fuse_options_extra";

arg_enum! {
    pub enum Cli {
        SCFS,
        SplitFS,
        CatFS,
    }
}

impl Cli {
    fn get_arguments<'a>(&self) -> Result<ArgMatches<'a>> {
        let app = match self {
            Cli::SCFS => app_scfs().args(&args_scfs()),
            Cli::SplitFS => app_splitfs().args(&args_splitfs()),
            Cli::CatFS => app_catfs().args(&args_catfs()),
        };
        app.get_matches_safe()
    }

    pub fn run(&self) {
        let arguments = self.get_arguments().unwrap_or_else(|e| e.exit());

        let mode = arguments.value_of(ARG_MODE);
        let mirror = arguments.value_of_os(ARG_MIRROR).unwrap();
        let mountpoint = arguments.value_of_os(ARG_MOUNTPOINT).unwrap();
        let blocksize = value_t!(arguments, ARG_BLOCKSIZE, u64);

        {
            let mirror = Path::new(mirror);
            let mountpoint = Path::new(mountpoint);

            if !mirror.exists() {
                panic!("Mirror path does not exist: {:?}", mirror)
            }

            if !mountpoint.exists() {
                panic!("Mountpoint path does not exist: {:?}", mountpoint)
            }

            if mirror
                .canonicalize()
                .unwrap()
                .starts_with(mountpoint.canonicalize().unwrap())
            {
                panic!(
                    "Mirror must not be in a subfolder of mountpoint: {:?}",
                    mountpoint
                )
            }
        }

        let fuse_options = arguments
            .values_of_os(ARG_FUSE_OPTIONS)
            .unwrap_or_default()
            .chain(
                arguments
                    .values_of_os(ARG_FUSE_OPTIONS_EXTRA)
                    .unwrap_or_default(),
            )
            .flat_map(|option| vec![OsStr::new("-o"), option]);

        let (tx_quitter, rx_quitter) = channel();

        ctrlc::set_handler(move || {
            tx_quitter.send(true).unwrap();
        })
        .expect("Error setting Ctrl-C handler");

        let _session = match (self, mode) {
            (Cli::CatFS, _) | (Cli::SCFS, Some("cat")) => {
                let fs = CatFS::new(mirror);
                mount(fs, &mountpoint, fuse_options)
            }

            (Cli::SplitFS, _) | (Cli::SCFS, Some("split")) => {
                let blocksize = blocksize.unwrap_or_else(|e| e.exit());
                let config = Config::default().blocksize(blocksize);
                let fs = SplitFS::new(mirror, config);
                mount(fs, &mountpoint, fuse_options)
            }

            _ => unreachable!(),
        };

        rx_quitter.recv().expect("Could not join quitter channel.");
    }
}

fn app_base<'a, 'b>() -> App<'a, 'b> {
    App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
}

fn app_scfs<'a, 'b>() -> App<'a, 'b> {
    app_base().about(crate_description!())
}

fn app_catfs<'a, 'b>() -> App<'a, 'b> {
    app_base().about("This is a convenience wrapper for the concatenating part of SCFS.")
}

fn app_splitfs<'a, 'b>() -> App<'a, 'b> {
    app_base().about("This is a convenience wrapper for the splitting part of SCFS.")
}

fn args_base<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![
        Arg::with_name(ARG_FUSE_OPTIONS)
            .short("o")
            .help("Additional options, which are passed down to FUSE")
            .multiple(true)
            .takes_value(true)
            .number_of_values(1),
        Arg::with_name(ARG_MIRROR)
            .help("Defines the directory that will be mirrored")
            .required(true),
        Arg::with_name(ARG_MOUNTPOINT)
            .help("Defines the mountpoint, where the mirror will be accessible")
            .required(true),
        Arg::with_name(ARG_FUSE_OPTIONS_EXTRA)
            .help("Additional options, which are passed down to FUSE")
            .multiple(true)
            .last(true),
    ]
}

fn args_scfs_only<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![Arg::with_name(ARG_MODE)
        .short(&ARG_MODE[0..1])
        .long(ARG_MODE)
        .help("Sets the desired mode")
        .takes_value(true)
        .possible_values(&["split", "cat"])
        .required(true)]
}

fn args_scfs<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    let mut args = args_base();
    args.append(args_catfs_only().as_mut());
    args.append(args_splitfs_only().as_mut());
    args.append(args_scfs_only().as_mut());
    args
}

fn args_catfs_only<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![]
}

fn args_catfs<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    let mut args = args_base();
    args.append(args_catfs_only().as_mut());
    args
}

fn args_splitfs_only<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![Arg::with_name(ARG_BLOCKSIZE)
        .short(&ARG_BLOCKSIZE[0..1])
        .long(ARG_BLOCKSIZE)
        .help("Sets the desired blocksize")
        .takes_value(true)
        .default_value("2097152")]
}

fn args_splitfs<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    let mut args = args_base();
    args.append(args_splitfs_only().as_mut());
    args
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn get_arguments() {
        for variant in &Cli::variants() {
            println!("Testing {:?}", variant);
            let variant = Cli::from_str(variant).unwrap();
            // This call must not panic.
            let _args = variant.get_arguments();
        }
    }
}
