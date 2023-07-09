use std::error::Error;

use scfs::Cli;

fn main() -> Result<(), Box<dyn Error>> {
    Cli::SCFS.run()
}
