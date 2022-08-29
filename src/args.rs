use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    /// The path to the configuration file.
    /// If the configuration file does not yet exist,
    /// a default configuration will be written.
    #[clap(short, long, default_value_t = String::from("/etc/simpleproxy/config.toml"), value_parser)]
    pub config: String,
    /// The verbosity level. The verbosity is determined by how often
    /// this flag is given.
    #[clap(short, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Args {
    pub fn parse() -> Self {
        Parser::parse()
    }
}
