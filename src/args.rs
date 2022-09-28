use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    /// The path to the configuration file.
    /// If the configuration file does not yet exist,
    /// a default configuration will be written.
    #[clap(short, long, default_value_t = String::from("/etc/simpleproxy/config.toml"), value_parser)]
    pub config: String,
}

impl Args {
    pub fn parse() -> Self {
        Parser::parse()
    }
}
