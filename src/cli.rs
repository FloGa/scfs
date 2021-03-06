use std::ffi::OsStr;
use std::fs;
use std::iter::FromIterator;
use std::path::Path;
use std::sync::mpsc::channel;

use clap::{
    arg_enum, crate_authors, crate_description, crate_name, crate_version, App, Arg, ArgMatches,
    Result,
};
use daemonize::Daemonize;

use crate::{mount, CatFS, Config, SplitFS, CONFIG_DEFAULT_BLOCKSIZE};

const ARG_MODE: &str = "mode";
const ARG_MIRROR: &str = "mirror";
const ARG_MOUNTPOINT: &str = "mountpoint";
const ARG_BLOCKSIZE: &str = "blocksize";
const ARG_FUSE_OPTIONS: &str = "fuse_options";
const ARG_FUSE_OPTIONS_EXTRA: &str = "fuse_options_extra";
const ARG_DAEMON: &str = "daemon";
const ARG_MKDIR: &str = "mkdir";

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
        let blocksize = arguments.value_of(ARG_BLOCKSIZE);
        let daemonize = arguments.is_present(ARG_DAEMON);
        let mkdir = arguments.is_present(ARG_MKDIR);

        let (mirror, mountpoint) = {
            let mirror = Path::new(mirror);
            let mountpoint = Path::new(mountpoint);

            if !mirror.exists() {
                panic!("Mirror path does not exist: {:?}", mirror)
            }

            if !mountpoint.exists() {
                if mkdir {
                    fs::create_dir_all(mountpoint).unwrap();
                } else {
                    panic!("Mountpoint path does not exist: {:?}", mountpoint)
                }
            }

            let mirror = mirror.canonicalize().unwrap();
            let mountpoint = mountpoint.canonicalize().unwrap();

            if mirror.starts_with(&mountpoint) {
                panic!(
                    "Mirror must not be in a subfolder of mountpoint: {:?}",
                    mountpoint
                )
            }

            (mirror.into_os_string(), mountpoint.into_os_string())
        };

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

        {
            let tx_quitter = tx_quitter.clone();
            ctrlc::set_handler(move || {
                tx_quitter.send(true).unwrap();
            })
            .expect("Error setting Ctrl-C handler");
        }

        let drop_hook = Box::new(move || {
            tx_quitter.send(true).unwrap_or(());
        });

        let _session = match (self, mode) {
            (Cli::CatFS, _) | (Cli::SCFS, Some("cat")) => {
                let fs = CatFS::new(&mirror, drop_hook);
                if daemonize {
                    Daemonize::new().start().expect("Failed to daemonize.");
                }
                mount(fs, &mountpoint, fuse_options)
            }

            (Cli::SplitFS, _) | (Cli::SCFS, Some("split")) => {
                let blocksize = blocksize.unwrap();
                let blocksize = convert_symbolic_quantity(blocksize).unwrap();
                let config = Config::default().blocksize(blocksize);
                let fs = SplitFS::new(&mirror, config, drop_hook);
                if daemonize {
                    Daemonize::new().start().expect("Failed to daemonize.");
                }
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
        Arg::with_name(ARG_DAEMON)
            .short(&ARG_DAEMON[0..1])
            .long(ARG_DAEMON)
            .help("Run program in background"),
        Arg::with_name(ARG_MKDIR)
            .long(ARG_MKDIR)
            .help("Create mountpoint directory if it does not exist already"),
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
    let default_blocksize = Box::new(CONFIG_DEFAULT_BLOCKSIZE.to_string());
    let default_blocksize: &'a String = Box::leak(default_blocksize);

    vec![Arg::with_name(ARG_BLOCKSIZE)
        .short(&ARG_BLOCKSIZE[0..1])
        .long(ARG_BLOCKSIZE)
        .help("Sets the desired blocksize")
        .takes_value(true)
        .default_value(&default_blocksize)]
}

fn args_splitfs<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    let mut args = args_base();
    args.append(args_splitfs_only().as_mut());
    args
}

fn convert_symbolic_quantity<S: Into<String>>(s: S) -> std::result::Result<u64, &'static str> {
    let s = s.into();
    let s = s.trim();
    let digits = String::from_iter(s.chars().take_while(|c| c.is_ascii_digit()).fuse());

    if digits.len() == 0 {
        return Err("No digits given");
    }

    let base = digits.parse::<u64>().unwrap();

    if base < 1 {
        return Err("Base may not be zero or negative");
    }

    let quantifier = s[digits.len()..].trim();

    if quantifier.len() > 1 || !"KMGT".contains(&quantifier) {
        return Err("Unkown quantifier");
    }

    let exp = match quantifier {
        "" => 0,
        "K" => 1,
        "M" => 2,
        "G" => 3,
        "T" => 4,
        _ => unreachable!(),
    };

    Ok(digits.parse::<u64>().unwrap() * 1024_u64.pow(exp))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_get_arguments_for_variant() {
        for variant in &Cli::variants() {
            println!("Testing {:?}", variant);
            let variant = Cli::from_str(variant).unwrap();
            // This call must not panic.
            let _args = variant.get_arguments();
        }
    }

    #[test]
    fn test_unknown_variant() -> std::result::Result<(), String> {
        match Cli::from_str("unknown") {
            Err(e) if e == "valid values: SCFS, SplitFS, CatFS" => Ok(()),
            Err(e) => Err(format!("Unexpected error: {}", e)),
            Ok(_) => Err(String::from("Did not result in Error")),
        }
    }

    #[test]
    fn test_symbolic_quantity_converter() {
        let sym_exp = vec![("", 0), ("K", 1), ("M", 2), ("G", 3), ("T", 4)];
        for (sym, exp) in sym_exp {
            println!("Testing 1{}", sym);
            assert_eq!(
                convert_symbolic_quantity(format!("1{}", sym)).unwrap(),
                1024_u64.pow(exp)
            );
        }
    }

    #[test]
    fn test_symbolic_quantity_converter_with_space() {
        assert_eq!(convert_symbolic_quantity(" 1024 ").unwrap(), 1024);
    }

    #[test]
    fn test_symbolic_quantity_converter_with_space_and_quantifier() {
        assert_eq!(convert_symbolic_quantity(" 1 K ").unwrap(), 1024);
    }

    #[test]
    fn test_symbolic_quantity_converter_fail_on_invalid_input() {
        assert!(convert_symbolic_quantity("1K1").is_err());
    }

    #[test]
    fn test_symbolic_quantity_converter_fail_on_empty_base() {
        assert!(convert_symbolic_quantity("K").is_err());
    }

    #[test]
    fn test_symbolic_quantity_converter_fail_on_zero() {
        assert!(convert_symbolic_quantity("0").is_err());
    }

    #[test]
    fn test_symbolic_quantity_converter_fail_on_negative() {
        assert!(convert_symbolic_quantity("-1").is_err());
    }
}
