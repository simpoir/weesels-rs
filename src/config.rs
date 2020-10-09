use serde::Deserialize;
use std::io::Read;
use std::{fs::File, path::PathBuf};

use crate::cli::CmdConf;
use crate::errors::Error;
use crate::errors::ErrorKind::ConfigError;

type Res<T> = Result<T, Error>;

const CONFIG_FILENAME: &str = "weesels.conf";

#[derive(Deserialize)]
pub struct Conf {
    pub host: String,
    pub port: u16,
    pub password: String,
    #[serde(default = "default_ssl")]
    pub ssl: bool,
    #[serde(default = "default_insecure")]
    pub insecure: bool,
}

fn default_ssl() -> bool {
    false
}

fn default_insecure() -> bool {
    false
}

pub struct Loader {
    prefix: PathBuf,
}

impl Loader {
    pub fn new() -> Res<Self> {
        Ok(Self {
            prefix: xdg::BaseDirectories::new()
                .map_err(|e| Error::from(ConfigError, e))?
                .get_config_home(),
        })
    }
    /// Load config from default path.
    pub fn load(&self, cli: &CmdConf) -> Res<Conf> {
        let default_path = self.prefix.join(CONFIG_FILENAME);
        let config_file = cli.config.as_ref().unwrap_or(&default_path);

        if cli.config.is_none() && !config_file.exists() {
            // conf_wizard()?;
        }

        load(config_file)
    }
}

/// Load config from specific path.
fn load(path: &std::path::Path) -> Res<Conf> {
    let mut f = File::open(path).map_err(|e| Error::from(ConfigError, e))?;
    let mut data = String::new();
    f.read_to_string(&mut data)
        .map_err(|e| Error::from(ConfigError, e))?;
    toml::from_str(&data).map_err(|e| Error::from(ConfigError, e))
}

#[cfg(test)]
mod tests {

    use super::*;
    use argh::FromArgs;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_load() {
        let mut f = NamedTempFile::new().unwrap();
        f.write(b"host='some.place'\nport=1235\npassword='flubar'\n")
            .unwrap();

        let c = super::load(f.into_temp_path().as_ref());
        let c = c.expect("should read config");
        assert_eq!("some.place", c.host);
        assert_eq!(1235, c.port);
        assert_eq!("flubar", c.password);
    }

    #[test]
    fn test_default_path() {
        let d = TempDir::new().unwrap().into_path();
        let mut f = std::fs::File::create(d.join(CONFIG_FILENAME)).unwrap();
        f.write(b"host='some.place'\nport=1235\npassword='flubar'\n")
            .unwrap();

        let c = CmdConf::from_args(&[], &[]).unwrap();
        let res = Loader { prefix: d }.load(&c).unwrap();
        assert_eq!("some.place", res.host);
    }

    #[test]
    fn test_load_cmdline() {
        let d = TempDir::new().unwrap().into_path();
        let dst = d.join(CONFIG_FILENAME);
        let mut f = std::fs::File::create(&dst).unwrap();
        f.write(b"host='some.place'\nport=1235\npassword='flubar'\n")
            .unwrap();

        let mut c = CmdConf::from_args(&[], &[]).unwrap();
        c.config = Some(dst);
        let res = Loader::new().unwrap().load(&c).unwrap();
        assert_eq!("some.place", res.host);
    }
}
