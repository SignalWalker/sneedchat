use rexa::locator::NodeLocator;
use std::{collections::HashMap, sync::Arc};

// pub mod chat;
mod cli;

mod netlayer;
use netlayer::*;

// mod netlayer;
// pub use netlayer::*;

// mod gui;

// #[cfg(feature = "iced")]
// fn main() -> iced::Result {
//     use iced::Application;
//
//     let args = <cli::Cli as clap::Parser>::parse();
//     cli::initialize_tracing(args.log_filter, args.log_format);
//     gui::SneedChat::run(iced::Settings {
//         window: iced::window::Settings {
//             size: iced::Size {
//                 width: 640.0,
//                 height: 480.0,
//             },
//             ..Default::default()
//         },
//         ..Default::default()
//     })
// }

// #[cfg(feature = "egui")]
// fn main() -> eframe::Result<()> {
//     let args = <cli::Cli as clap::Parser>::parse();
//     cli::initialize_tracing(args.log_filter, args.log_format);
//     let options = eframe::NativeOptions {
//         viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
//         renderer: eframe::Renderer::Wgpu,
//         ..Default::default()
//     };
//     eframe::run_native(
//         "SneedChat",
//         options,
//         Box::new(|cc| {
//             egui_extras::install_image_loaders(&cc.egui_ctx);
//             Box::<SneedChat>::default()
//         }),
//     )
// }

// #[cfg(all(not(feature = "iced"), not(feature = "egui")))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let args = <cli::Cli as clap::Parser>::parse();
    cli::initialize_tracing(args.log_filter, args.log_format);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .unhandled_panic(tokio::runtime::UnhandledPanic::ShutdownRuntime)
        .enable_all()
        .build()?;

    let local_id = uuid::Uuid::new_v4();
    let username = args.username.unwrap_or_else(|| local_id.to_string());

    let (ev_sender, ev_receiver) = tokio::sync::mpsc::unbounded_channel();

    let (end_notifier, mut end_flag) = tokio::sync::watch::channel(false);
    end_flag.mark_unchanged();

    rt.block_on(async move {
        let layers = {
            let mut layers = NetlayerManager::new();
            let tcpip =
                rexa::netlayer::datastream::TcpIpNetlayer::bind(&([0, 0, 0, 0], args.port).into())
                    .await?;
            layers.register(
                "tcpip".to_owned(),
                tcpip,
                ev_sender.clone(),
                end_flag.clone(),
            );
            let mock = rexa::netlayer::mock::MockNetlayer::bind(username.clone())?;
            layers.register("mock".to_owned(), mock, ev_sender.clone(), end_flag.clone());
            Arc::new(layers)
        };

        let mut main_tasks = tokio::task::JoinSet::<
            Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>,
        >::new();

        let input_task = handle_input(ev_sender.clone())?;

        main_tasks
            .build_task()
            .name("signal_watch")
            .spawn(futures::FutureExt::map(
                signal_watch(ev_sender.clone(), end_flag.clone()),
                |res| res.map_err(From::from),
            ))?;

        if let Some(remote) = args.remote {
            ev_sender.send(SneedEvent::Connect(
                NodeLocator::<String, syrup::Item>::new(remote.to_string(), "tcpip".to_owned()),
            ));
        }

        tracing::info!("ready!");

        tokio::task::Builder::new()
            .name("main_loop")
            .spawn(main_loop(
                layers,
                ev_receiver,
                ev_sender,
                username,
                local_id,
            ))?
            .await??;

        tracing::info!("exited main loop, awaiting subtasks...");

        end_notifier.send(true)?;
        while let Some(res) = main_tasks.join_next().await {
            if let Err(error) = res {
                tracing::error!("{error}");
            }
        }

        if input_task.is_finished() {
            match input_task.join() {
                Ok(Err(inner)) => {
                    tracing::error!(name: "sneedchat::handle_input::inner", "{inner}")
                }
                Err(outer) => tracing::error!(name: "sneedchat::handle_input", "{outer:?}"),
                _ => {}
            }
        }

        tracing::info!("exiting sneedchat...");

        async fn main_loop(
            layers: Arc<NetlayerManager>,
            mut ev_receiver: tokio::sync::mpsc::UnboundedReceiver<SneedEvent>,
            ev_sender: tokio::sync::mpsc::UnboundedSender<SneedEvent>,
            username: String,
            local_id: uuid::Uuid,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
            let time_offset = time::UtcOffset::current_local_offset().unwrap_or_else(|e| {
                tracing::warn!(reason = %e, "couldn't get local time offset");
                time::UtcOffset::UTC
            });
            let time_format = time::macros::format_description!("[hour]:[minute]:[second]");

            let mut tasks = tokio::task::JoinSet::new();

            let mut users = HashMap::<uuid::Uuid, User>::new();
            let mut channels = HashMap::<uuid::Uuid, Arc<Channel>>::new();
            let outboxes = Outboxes::default();
            loop {
                tracing::trace!("awaiting chat event");
                let Some(event) = ev_receiver.recv().await else {
                    tracing::error!("ev_receiver broken");
                    break;
                };
                tracing::debug!(?event, "received event");
                match event {
                    SneedEvent::Connect(locator) => {
                        async fn connect(
                            layers: Arc<NetlayerManager>,
                            locator: NodeLocator<String, syrup::Item>,
                            local_id: uuid::Uuid,
                            outboxes: Arc<dashmap::DashMap<uuid::Uuid, Outbox>>,
                        ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
                        {
                            let session = match layers.request_connect(locator.clone())?.await? {
                                Ok(session) => session,
                                Err(_) => {
                                    tracing::error!(?locator, "could not connect to session");
                                    return Err(todo!());
                                }
                            };
                            let outbox = match session
                                .into_remote_bootstrap()
                                .fetch(local_id.as_bytes())
                                .await?
                                .await
                            {
                                Ok(base) => Outbox::new(base),
                                Err(reason) => todo!(),
                            };
                            let profile = outbox.get_profile().await?;
                            outboxes.insert(profile.id, outbox);
                            Ok(())
                        }
                        let _ = tasks.build_task().name("connect").spawn(connect(
                            layers.clone(),
                            locator,
                            local_id,
                            outboxes.clone(),
                        ));
                    }
                    SneedEvent::Fetch(session, swiss, resolver) => {
                        let id = uuid::Uuid::from_slice(&swiss)?;
                        let (user, channel) = User::new(
                            Profile {
                                id: local_id,
                                username: username.clone(),
                            },
                            session.clone(),
                            id,
                            ev_sender.clone(),
                        );
                        users.insert(id, user);
                        channels.insert(id, channel.clone());
                        resolver.send(channel).unwrap();
                        if !outboxes.contains_key(&id) {
                            tasks
                                .build_task()
                                .name("fetch_outbox_fetch")
                                .spawn(fetch_outbox(
                                    username.clone(),
                                    local_id,
                                    id,
                                    outboxes.clone(),
                                    session,
                                ))?;
                        }
                    }
                    SneedEvent::RecvMessage(channel_id, msg) => {
                        let user = users.get(&channel_id).unwrap();
                        let time = time::OffsetDateTime::now_utc()
                            .to_offset(time_offset)
                            .format(&time_format)
                            .unwrap();
                        println!("[{time}][{}] {msg}", user.name);
                    }
                    SneedEvent::SetName(channel_id, name) => {
                        tracing::trace!(%name, %channel_id, "update username");
                        users.get_mut(&channel_id).unwrap().name = name;
                    }
                    SneedEvent::SendMessage(msg) => {
                        for entry in outboxes.iter() {
                            let outbox = entry.value();
                            outbox.message(&msg).await?;
                        }
                        let time = time::OffsetDateTime::now_utc()
                            .to_offset(time_offset)
                            .format(&time_format)
                            .unwrap();
                        println!("[{time}][{username}] {msg}");
                    }
                    SneedEvent::Exit => {
                        // for entry in outboxes.iter() {
                        //     let outbox = entry.value();
                        //     outbox.abort("quitting").await?;
                        // }
                        for (_, user) in users.drain() {
                            if !user.session.is_aborted() {
                                user.session.abort("quitting".to_string()).await?;
                            }
                        }
                        tracing::debug!("received exit request");
                        break;
                    }
                }
            }
            tracing::trace!("ending main loop...");
            tasks.abort_all();
            while let Some(res) = tasks.join_next().await {
                res??;
            }
            Ok(())
        }

        Ok(())
    })
}
