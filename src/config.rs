use serde::Deserialize;
use std::fs::File;
use std::io::Read;

use crate::errors::Error;
use crate::errors::ErrorKind::ConfigError;

type Res<T> = Result<T, Error>;

#[derive(Deserialize)]
pub struct Conf {
    pub host: String,
    pub port: u16,
    pub password: String,
    #[serde(default="default_ssl")]
    pub ssl: bool,
}

fn default_ssl() -> bool {
    false
}

pub fn load_default() -> Res<Conf> {
    let cf = xdg::BaseDirectories::new()
        .unwrap()
        .find_config_file("weesels.conf");
    load(cf.ok_or_else(|| Error::new(ConfigError))?)
}

pub fn load(path: std::path::PathBuf) -> Res<Conf> {
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

        let c = super::load(f.into_temp_path().to_path_buf());
        let c = c.expect("should read config");
        assert_eq!("some.place", c.host);
        assert_eq!(1235, c.port);
        assert_eq!("flubar", c.password);
    }
}
