use std::{future::Future, sync::Arc};

use ed25519_dalek::SigningKey;
use rexa::netlayer::Netlayer;
use tokio::{sync::watch, task::JoinSet};

use crate::{
    ChatData, ChatManager, EventReceiver, EventSender, Gateway, NetlayerManager, Persona, Profile,
};

pub struct ChatManagerBuilder {
    signing_key: SigningKey,
    layers: NetlayerManager,
    ev_sender: EventSender,
    ev_receiver: EventReceiver,
    end_flag: watch::Receiver<bool>,
    end_notifier: watch::Sender<bool>,
    subscription_tasks: JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>,
    username: Option<String>,
    avatar: Option<String>,
}

impl ChatManagerBuilder {
    pub(super) fn new(signing_key: SigningKey) -> Self {
        let (ev_sender, ev_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (end_notifier, mut end_flag) = tokio::sync::watch::channel(false);
        end_flag.mark_unchanged();

        Self {
            signing_key,
            layers: NetlayerManager::new(),
            ev_sender,
            ev_receiver,
            end_notifier,
            end_flag,
            subscription_tasks: JoinSet::new(),
            username: None,
            avatar: None,
        }
    }

    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_avatar(mut self, avatar: String) -> Self {
        self.avatar = Some(avatar);
        self
    }

    pub fn with_netlayer<Nl>(mut self, transport: String, netlayer: Nl) -> Self
    where
        Nl: Netlayer + Send + 'static,
        Nl::Reader: rexa::async_compat::AsyncRead + Unpin + Send,
        Nl::Writer: rexa::async_compat::AsyncWrite + Unpin + Send,
        Nl::Error: std::error::Error + Send + Sync + 'static,
    {
        self.layers.register(
            transport,
            netlayer,
            self.ev_sender.clone(),
            self.end_flag.clone(),
        );
        self
    }

    pub fn subscribe<
        F: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>>
            + Send
            + 'static,
    >(
        mut self,
        producer: impl FnOnce(EventSender, watch::Receiver<bool>) -> F,
    ) -> Self {
        self.subscription_tasks
            .spawn(producer(self.ev_sender.clone(), self.end_flag.clone()));
        self
    }

    pub fn subscribe_threaded<Res>(self, producer: impl FnOnce(EventSender) -> Res) -> (Self, Res) {
        let res = producer(self.ev_sender.clone());
        (self, res)
    }

    pub fn build(self) -> ChatManager {
        let skey = self.signing_key;
        let vkey = skey.verifying_key();
        let username = self
            .username
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let data = Arc::new(ChatData::default());
        let persona = Arc::new(Persona::new(Profile::new(vkey, username, self.avatar)));
        ChatManager {
            signing_key: Arc::new(parking_lot::RwLock::new(skey)),
            persona,

            layers: Arc::new(self.layers),
            ev_sender: self.ev_sender.clone(),
            ev_receiver: self.ev_receiver.into(),
            end_notifier: self.end_notifier,
            subscription_tasks: self.subscription_tasks.into(),

            data,

            gateway: Arc::new(Gateway::new(self.ev_sender)),
            portals: Default::default(),
            channels: Default::default(),
        }
    }
}
