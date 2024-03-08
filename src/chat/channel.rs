use crate::chat::{EventSender, Profile, SneedEvent};
use futures::future::BoxFuture;
use rexa::captp::AbstractCapTpSession;
use std::sync::Arc;

// pub struct Channel {
//     pub(super) profile: Profile,
//     pub(super) id: uuid::Uuid,
//     pub(super) ev_pipe: EventSender,
//     pub(super) sessions:
//         Arc<tokio::sync::RwLock<Vec<(u64, Arc<dyn AbstractCapTpSession + Send + Sync + 'static>)>>>,
// }

// impl std::fmt::Debug for Channel {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("Channel")
//             .field("profile", &self.profile)
//             .field("id", &self.id)
//             // .field("sessions", &self.sessions)
//             .finish_non_exhaustive()
//     }
// }
//
// impl std::clone::Clone for Channel {
//     fn clone(&self) -> Self {
//         Self {
//             profile: self.profile.clone(),
//             id: self.id,
//             ev_pipe: self.ev_pipe.clone(),
//             sessions: self.sessions.clone(),
//         }
//     }
// }
//
// impl rexa::captp::object::Object for Channel {
//     fn deliver_only(
//         &self,
//         args: Vec<syrup::Item>,
//     ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
//         tracing::debug!("channel::deliver_only");
//         let mut args = args.into_iter();
//         match args.next() {
//             Some(syrup::Item::Symbol(id)) => match id.as_str() {
//                 "message" => match args.next() {
//                     Some(syrup::Item::String(msg)) => {
//                         self.ev_pipe.send(SneedEvent::RecvMessage(self.id, msg))?;
//                         Ok(())
//                     }
//                     Some(s) => Err(format!("invalid message text: {s:?}").into()),
//                     None => Err("missing message text".into()),
//                 },
//                 "set_name" => match args.next() {
//                     Some(syrup::Item::String(name)) => {
//                         self.ev_pipe.send(SneedEvent::SetName(self.id, name))?;
//                         Ok(())
//                     }
//                     Some(s) => Err(format!("invalid username: {s:?}").into()),
//                     None => Err("missing username".into()),
//                 },
//                 id => Err(format!("unrecognized channel function: {id}").into()),
//             },
//             Some(s) => Err(format!("invalid deliver_only argument: {s:?}").into()),
//             None => Err("missing deliver_only arguments".into()),
//         }
//     }
//
//     fn deliver<'s>(
//         &'s self,
//         args: Vec<syrup::Item>,
//         resolver: rexa::captp::GenericResolver,
//     ) -> BoxFuture<'s, Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>> {
//         use futures::FutureExt;
//         async move {
//             tracing::debug!("channel::deliver");
//             let mut args = args.into_iter();
//             match args.next() {
//                 Some(syrup::Item::Symbol(id)) => match id.as_str() {
//                     "get_profile" => resolver
//                         .fulfill(
//                             [&self.profile],
//                             None,
//                             rexa::captp::msg::DescImport::Object(0.into()),
//                         )
//                         .await
//                         .map_err(From::from),
//                     id => Err(format!("unrecognized channel function: {id}").into()),
//                 },
//                 Some(s) => Err(format!("invalid deliver argument: {s:?}").into()),
//                 None => Err("missing deliver arguments".into()),
//             }
//         }
//         .boxed()
//     }
// }
