#[cfg(not(target_family = "wasm"))]
mod desktop {
    use crate::cfg::Config;
    use directories::ProjectDirs;
    use ed25519_dalek::{
        pkcs8::{DecodePrivateKey, EncodePrivateKey},
        SigningKey,
    };
    use figment::{
        providers::{Format, Toml},
        Figment,
    };
    use rand::rngs::OsRng;
    use std::{
        net::{IpAddr, Ipv4Addr},
        path::{Path, PathBuf},
    };

    lazy_static::lazy_static! {
        pub(crate) static ref PROJECT_DIRS: ProjectDirs = ProjectDirs::from("org", "Signal Garden", "Bonfire").expect("could not get user home directory");
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(default)]
    pub(crate) struct Directories {
        cache: PathBuf,
        data: PathBuf,
        config: PathBuf,
    }

    impl Default for Directories {
        fn default() -> Self {
            Self {
                cache: PROJECT_DIRS.cache_dir().to_owned(),
                data: PROJECT_DIRS.data_dir().to_owned(),
                config: PROJECT_DIRS.config_dir().to_owned(),
            }
        }
    }

    #[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
    #[serde(default)]
    pub(crate) struct Desktop {
        key_file: PathBuf,
        directories: Directories,
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(default)]
    pub(crate) struct TcpIpConfig {
        listen_address: IpAddr,
        port: u16,
    }

    impl Default for TcpIpConfig {
        fn default() -> Self {
            Self {
                listen_address: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
                port: 43456,
            }
        }
    }

    impl TcpIpConfig {
        pub(crate) fn socket_addr(&self) -> std::net::SocketAddr {
            std::net::SocketAddr::new(self.listen_address, self.port)
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub(crate) enum WriteError {
        #[error(transparent)]
        Io(#[from] std::io::Error),
        #[error(transparent)]
        Serialize(#[from] toml::ser::Error),
        #[error(transparent)]
        Key(#[from] ed25519_dalek::pkcs8::Error),
    }

    impl Config {
        pub(crate) fn read(config_dir: impl AsRef<Path>) -> Result<Self, figment::Error> {
            let config_dir = config_dir.as_ref();
            let mut res: Config = Figment::new()
                .merge(Toml::file(config_dir.join("config.toml")))
                .extract()?;
            config_dir.clone_into(&mut res.desktop.directories.config);
            if !res.desktop.key_file.has_root() {
                res.desktop.key_file = res.desktop.directories.data.join("ed25519.sign");
            }
            Ok(res)
        }

        pub(crate) fn write(&self) -> Result<(), WriteError> {
            let cfg_path = self.desktop.directories.config.join("config.toml");
            std::fs::write(cfg_path, toml::to_string(self)?).map_err(From::from)
        }

        pub(crate) fn get_key_or_init(&self) -> Result<SigningKey, WriteError> {
            let path = &self.desktop.key_file;
            if path.try_exists()? {
                SigningKey::read_pkcs8_der_file(path).map_err(From::from)
            } else {
                tracing::warn!(?path, "key file does not exist; generating new");
                if let Some(dir) = path.parent() {
                    if !dir.try_exists()? {
                        tracing::warn!(?dir, "creating directory");
                        std::fs::create_dir_all(dir)?;
                    }
                }
                let key = SigningKey::generate(&mut OsRng);
                SigningKey::write_pkcs8_der_file(&key, path)?;
                Ok(key)
            }
        }
    }
}
#[cfg(not(target_family = "wasm"))]
pub(crate) use desktop::*;

#[cfg(target_family = "wasm")]
mod web {
    pub(crate) struct Platform {}
    impl Config {}
}
#[cfg(target_family = "wasm")]
pub(crate) use web::*;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(default)]
pub(crate) struct Profile {
    pub(crate) username: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            username: "<Unknown>".to_owned(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) profile: Profile,
    pub(crate) netlayers: NetlayerConfig,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) desktop: Desktop,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
#[serde(default)]
pub(crate) struct NetlayerConfig {
    #[cfg(not(target_family = "wasm"))]
    pub(crate) tcpip: TcpIpConfig,
}
