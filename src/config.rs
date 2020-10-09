use serde::Deserialize;
use std::fs::File;
use std::io::Read;

use crate::cli::CmdConf;
use crate::errors::Error;
use crate::errors::ErrorKind::ConfigError;

type Res<T> = Result<T, Error>;

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

/// Load config from default path.
pub fn load_default(cli: &CmdConf) -> Res<Conf> {
    let default_path;
    let config = match &cli.config {
        Some(path) => path,
        None => {
            default_path = xdg::BaseDirectories::new()
                .map_err(|e| Error::from(ConfigError, e))?
                .find_config_file("weesels.conf")
                .ok_or_else(|| Error::new(ConfigError))?;
            &default_path
        }
    };

    load(config)
}

/// Load config from specific path.
pub fn load(path: &std::path::Path) -> Res<Conf> {
    let mut f = File::open(path).map_err(|e| Error::from(ConfigError, e))?;
    let mut data = String::new();
    f.read_to_string(&mut data)
        .map_err(|e| Error::from(ConfigError, e))?;
    toml::from_str(&data).map_err(|e| Error::from(ConfigError, e))
}

#[cfg(test)]
mod tests {

    use std::io::Write;
    use tempfile::NamedTempFile;

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
}
