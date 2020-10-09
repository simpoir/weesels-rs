use std::path::PathBuf;

use argh::FromArgs;

#[derive(FromArgs)]
#[argh(description = "TUI client for the Weechat relay plugin.")]
pub struct CmdConf {
    /// verbose logging
    #[argh(switch, short = 'v')]
    pub verbosity: i8,

    /// log path
    #[argh(option)]
    pub log_file: Option<PathBuf>,

    /// path to config file
    #[argh(option)]
    pub config: Option<PathBuf>,
}

impl CmdConf {
    pub fn from_env() -> Self {
        argh::from_env()
    }
}
