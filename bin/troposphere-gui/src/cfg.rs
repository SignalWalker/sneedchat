#[cfg(not(target_family = "wasm"))]
mod desktop {
    use crate::cfg::Config;
    use directories::ProjectDirs;
    use ed25519_dalek::{
        pkcs8::{DecodePrivateKey, EncodePrivateKey},
        SigningKey, VerifyingKey,
    };
    use figment::{
        providers::{Format, Toml},
        Figment,
    };
    use rand::rngs::OsRng;
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        path::{Path, PathBuf},
    };

    lazy_static::lazy_static! {
        pub(crate) static ref PROJECT_DIRS: ProjectDirs = ProjectDirs::from("org", "Signal Garden", "Troposphere").expect("could not get user home directory");
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
    #[serde(default)]
    pub(crate) struct Directories {
        pub(crate) cache: PathBuf,
        pub(crate) data: PathBuf,
        pub(crate) config: PathBuf,
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

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
    pub(crate) struct MdnsConfig {
        pub(crate) broadcast: bool,
        pub(crate) listen: bool,
    }

    impl Default for MdnsConfig {
        fn default() -> Self {
            Self {
                broadcast: true,
                listen: true,
            }
        }
    }

    #[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone)]
    #[serde(default)]
    pub(crate) struct Desktop {
        pub(crate) key_file: PathBuf,
        pub(crate) directories: Directories,
        pub(crate) mdns: MdnsConfig,
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
    #[serde(default)]
    pub(crate) struct TcpIpConfig {
        pub(crate) listen_addresses: Vec<IpAddr>,
        pub(crate) port: u16,
    }

    impl Default for TcpIpConfig {
        fn default() -> Self {
            Self {
                listen_addresses: vec![
                    IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                ],
                port: 0,
            }
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
        pub(crate) fn read(
            args: crate::cli::Cli,
            config_dir: impl AsRef<Path>,
        ) -> Result<Self, figment::Error> {
            let config_dir = config_dir.as_ref();
            let mut res: Config = Figment::new()
                .merge(Toml::file(config_dir.join("config.toml")))
                .extract()?;
            config_dir.clone_into(&mut res.desktop.directories.config);
            // -- merge cli args --
            if let Some(cache_dir) = args.cache_dir {
                res.desktop.directories.cache = cache_dir;
            }
            if let Some(data_dir) = args.data_dir {
                res.desktop.directories.data = data_dir;
            }
            // -- end merge cli args --
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
                tracing::debug!(?path, "found key file");
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
    use crate::gui::chat::ChatError;

    use super::{Config, Profile};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[derive(Debug, thiserror::Error)]
    pub(crate) enum WriteError {
        #[error(transparent)]
        Io(#[from] std::io::Error),
        // #[error(transparent)]
        // Serialize(#[from] toml::ser::Error),
        #[error(transparent)]
        Key(#[from] ed25519_dalek::pkcs8::Error),
    }

    #[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
    pub(crate) struct Web {
        signing_key: SigningKey,
    }

    impl Default for Web {
        fn default() -> Self {
            Self {
                signing_key: SigningKey::generate(&mut OsRng),
            }
        }
    }

    impl Config {
        pub(crate) fn read() -> Result<Self, figment::Error> {
            Ok(Self {
                profile: Default::default(),
                netlayers: Default::default(),
                web: Web {
                    signing_key: SigningKey::generate(&mut OsRng),
                },
            })
        }

        pub(crate) fn get_key_or_init(&self) -> Result<SigningKey, WriteError> {
            Ok(self.web.signing_key.clone())
        }
    }
}
#[cfg(target_family = "wasm")]
pub(crate) use web::*;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
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

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) profile: Profile,
    pub(crate) netlayers: NetlayerConfig,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) desktop: Desktop,
    #[cfg(target_family = "wasm")]
    pub(crate) web: Web,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub(crate) struct NetlayerConfig {
    #[cfg(not(target_family = "wasm"))]
    pub(crate) tcpip: TcpIpConfig,
}
