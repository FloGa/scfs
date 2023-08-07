use std::path::PathBuf;

use assert_cmd::Command;
use lazy_static::lazy_static;
use predicates::prelude::*;

lazy_static! {
    static ref SCFS_PATH: PathBuf = assert_cmd::cargo::cargo_bin("scfs");
    static ref SPLITFS_PATH: PathBuf = assert_cmd::cargo::cargo_bin("splitfs");
    static ref CATFS_PATH: PathBuf = assert_cmd::cargo::cargo_bin("catfs");
}

#[test]
fn help_works() {
    Command::new(&*SCFS_PATH)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("\nUsage: scfs"));

    for mode in "split cat".split_whitespace() {
        Command::new(&*SCFS_PATH)
            .arg(mode)
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("\nUsage: scfs {}", mode)));
    }

    Command::new(&*SPLITFS_PATH)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("\nUsage: splitfs"));

    Command::new(&*CATFS_PATH)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("\nUsage: catfs"));
}

#[test]
fn correct_version() {
    let version = env!("CARGO_PKG_VERSION");

    Command::new(&*SCFS_PATH)
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("scfs {}\n", version));

    for mode in "split cat".split_whitespace() {
        Command::new(&*SCFS_PATH)
            .arg(mode)
            .arg("--version")
            .assert()
            .success()
            .stdout(format!("scfs-{} {}\n", mode, version));
    }

    Command::new(&*SPLITFS_PATH)
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("splitfs {}\n", version));

    Command::new(&*CATFS_PATH)
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("catfs {}\n", version));
}
