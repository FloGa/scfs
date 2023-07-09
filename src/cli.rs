use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::iter::FromIterator;
use std::path::PathBuf;
use std::sync::mpsc::channel;

use clap::{Args, Parser, Subcommand};
use daemonize::Daemonize;

use crate::{mount, CatFS, Config, SplitFS, CONFIG_DEFAULT_BLOCKSIZE};

pub enum Cli {
    SCFS,
    SplitFS,
    CatFS,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct CommandScfs {
    #[command(flatten)]
    args: ArgsScfs,
}

#[derive(Parser)]
#[command(author, version, long_about = None)]
#[command(about = "This is a convenience wrapper for the splitting part of SCFS.")]
struct CommandSplitFs {
    #[command(flatten)]
    args: ArgsSplit,
}

#[derive(Parser)]
#[command(author, version, long_about = None)]
#[command(about = "This is a convenience wrapper for the concatenating part of SCFS.")]
struct CommandCatFs {
    #[command(flatten)]
    args: ArgsCat,
}

#[derive(Debug, Subcommand)]
enum Mode {
    /// Create a splitting file system
    Split(ArgsSplit),

    /// Create a concatenating file system
    Cat(ArgsCat),
}

#[derive(Args, Debug)]
struct ArgsScfs {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Args, Debug)]
struct ArgsCommon {
    /// Defines the directory that will be mirrored
    mirror: PathBuf,

    /// Defines the mountpoint, where the mirror will be accessible
    mountpoint: PathBuf,

    /// Additional options, which are passed down to FUSE
    #[arg(long, short = 'o')]
    fuse_options: Vec<OsString>,

    /// Run program in background
    #[arg(long, short = 'd')]
    daemon: bool,

    /// Create mountpoint directory if it does not exist already
    #[arg(long)]
    mkdir: bool,

    /// Additional options, which are passed down to FUSE
    #[arg(last = true)]
    fuse_options_extra: Vec<OsString>,
}

#[derive(Args, Debug)]
struct ArgsSplit {
    /// Sets the desired blocksize
    #[arg(long, short = 'b', value_parser = convert_symbolic_quantity, default_value_t = CONFIG_DEFAULT_BLOCKSIZE)]
    blocksize: u64,

    #[command(flatten)]
    args_common: ArgsCommon,
}

#[derive(Args, Debug)]
struct ArgsCat {
    #[command(flatten)]
    args_common: ArgsCommon,
}

impl Cli {
    pub fn run(&self) -> Result<(), Box<dyn Error>> {
        let mode = match self {
            Cli::SCFS => CommandScfs::parse().args.mode,
            Cli::SplitFS => Mode::Split(CommandSplitFs::parse().args),
            Cli::CatFS => Mode::Cat(CommandCatFs::parse().args),
        };

        let args_common = match &mode {
            Mode::Split(args) => &args.args_common,
            Mode::Cat(args) => &args.args_common,
        };

        let (mirror, mountpoint) = {
            let mirror = &args_common.mirror;
            let mountpoint = &args_common.mountpoint;

            if !mirror.exists() {
                panic!("Mirror path does not exist: {:?}", mirror)
            }

            if !mountpoint.exists() {
                if args_common.mkdir {
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

        let fuse_options = &args_common.fuse_options;
        let fuse_options_extra = &args_common.fuse_options_extra;

        let fuse_options = fuse_options
            .iter()
            .chain(fuse_options_extra.iter())
            .flat_map(|option| vec![OsStr::new("-o"), &option]);

        if args_common.daemon {
            Daemonize::new().start().expect("Failed to daemonize.");
        }

        let _session = match &mode {
            Mode::Split(args) => {
                let blocksize = args.blocksize;
                let config = Config::default().blocksize(blocksize);
                let fs = SplitFS::new(&mirror, config, drop_hook);
                mount(fs, &mountpoint, fuse_options)
            }

            Mode::Cat(_args) => {
                let fs = CatFS::new(&mirror, drop_hook);
                mount(fs, &mountpoint, fuse_options)
            }
        };

        rx_quitter.recv().expect("Could not join quitter channel.");

        Ok(())
    }
}

fn convert_symbolic_quantity(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let digits = String::from_iter(s.chars().take_while(|c| c.is_ascii_digit()).fuse());

    if digits.len() == 0 {
        return Err(String::from("No digits given"));
    }

    let base = digits.parse::<u64>().unwrap();

    if base < 1 {
        return Err(String::from("Base may not be zero or negative"));
    }

    let quantifier = s[digits.len()..].trim();

    let exp = match quantifier {
        "" => 0,
        "K" => 1,
        "M" => 2,
        "G" => 3,
        "T" => 4,
        _ => return Err(String::from("Unkown quantifier")),
    };

    Ok(digits.parse::<u64>().unwrap() * 1024_u64.pow(exp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbolic_quantity_converter() {
        let sym_exp = vec![("", 0), ("K", 1), ("M", 2), ("G", 3), ("T", 4)];
        for (sym, exp) in sym_exp {
            println!("Testing 1{}", sym);
            assert_eq!(
                convert_symbolic_quantity(format!("1{}", sym).as_str()).unwrap(),
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
