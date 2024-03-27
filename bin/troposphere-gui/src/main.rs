#![feature(strict_provenance)]
#![feature(multiple_supertrait_upcastable)]
#![feature(must_not_suspend)]
#![feature(fs_try_exists)]
#![feature(ip)]

use dioxus::hooks::use_coroutine;

use crate::cfg::Config;

mod cfg;
#[cfg(not(target_family = "wasm"))]
mod cli;
mod gui;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod native;

fn main() {
    let cfg;
    #[cfg(not(target_family = "wasm"))]
    {
        let args = <cli::Cli as clap::Parser>::parse();
        cli::initialize_tracing(args.log_filter.clone(), args.log_format);
        let config_dir = args
            .config_dir
            .as_deref()
            .unwrap_or_else(|| crate::cfg::PROJECT_DIRS.config_dir())
            .to_owned();
        tracing::info!(configuration_directory = ?config_dir, "reading config");
        if !std::fs::try_exists(&config_dir)
            .expect("could not confirm/deny existence of configuration directory")
        {
            tracing::warn!(configuration_directory = ?config_dir, "config dir does not exist");
            std::fs::create_dir_all(&config_dir).expect("could not create configuration directory");
        }
        cfg = Config::read(args, config_dir).expect("could not read config");
    }

    #[cfg(target_family = "wasm")]
    {
        cfg = Config::read().expect("could not read config")
    }

    gui::run(cfg);
}

pub(crate) fn spawn_coroutine<M, G, F>(init: G) -> dioxus::hooks::Coroutine<M>
where
    M: 'static,
    G: FnOnce(dioxus::hooks::UnboundedReceiver<M>) -> F,
    F: std::future::Future<Output = ()> + Send + 'static,
{
    #[cfg(not(target_family = "wasm"))]
    {
        use_coroutine(move |rx| {
            let future = init(rx);
            async move {
                if let Err(error) = tokio::spawn(future).await {
                    tracing::error!(%error, "failed to join spawned coroutine");
                }
            }
        })
    }
    #[cfg(target_family = "wasm")]
    {
        // dioxus-web does not use tokio
        use_coroutine(init)
    }
}
