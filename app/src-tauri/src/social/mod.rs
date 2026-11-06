//! Friends, messages and world invites, behind one provider so the UI, the bell and the sounds never know where they
//! come from: Essential's social system (if that collaboration happens) or Obelisk's own. Off by default (no provider);
//! the mock provider stands in while the UI and sounds are built. Outline: obelisk-social/OUTLINE.md.

mod essential;
mod mock;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    pub id: String,
    pub name: String,
    /// The Minecraft account's UUID when known (skins, joining their world).
    pub uuid: Option<String>,
    pub avatar: Option<String>,
    pub presence: Presence,
    pub since: Option<i64>,
}

/// What a friend is doing. `pack` names what they play, so an invite can check you have the same thing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Presence {
    #[default]
    Offline,
    /// Obelisk is open, no game running.
    Online,
    Playing { pack: Option<PackRef>, world: Option<String>, joinable: bool },
}

/// Enough to find or fetch the same pack: an installed profile matches on name, version, loader and mod-list hash.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackRef {
    pub name: String,
    pub mc: String,
    pub loader: String,
    pub mods_hash: Option<String>,
    /// Where to get it when it isn't installed (a share link, a Modrinth/CurseForge pack id).
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation: String,
    pub from: String,
    pub text: String,
    pub sent: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub members: Vec<String>,
    pub unread: u32,
    pub last: Option<Message>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub id: String,
    pub from: String,
    pub pack: Option<PackRef>,
    /// What to join: a world being shared, a server address, or a LAN target the provider hands over.
    pub target: String,
    pub expires: Option<i64>,
}

/// Everything that can happen, as the UI hears it (`social-event`). Each maps to a sound cue of the same name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SocialEvent {
    FriendRequest { from: Friend },
    FriendAccepted { friend: Friend },
    FriendOnline { id: String },
    FriendOffline { id: String },
    Presence { id: String, presence: Presence },
    Message { message: Message },
    MessageSent { message: Message },
    Mention { message: Message },
    Invite { invite: Invite },
    InviteAccepted { invite: String, by: String },
    InviteDeclined { invite: String, by: String },
    JoinedWorld { id: String },
    Connected,
    Disconnected { why: String },
    Reconnected,
    Error { text: String },
}

impl SocialEvent {
    /// The sound cue (obelisk sound's `sound::cue`); None stays silent.
    pub fn cue(&self) -> Option<&'static str> {
        Some(match self {
            Self::FriendRequest { .. } => "friend_request",
            Self::FriendAccepted { .. } => "friend_accepted",
            Self::FriendOnline { .. } => "friend_online",
            Self::Message { .. } => "message",
            Self::MessageSent { .. } => "message_sent",
            Self::Mention { .. } => "mention",
            Self::Invite { .. } => "invite",
            Self::InviteAccepted { .. } => "invite_accepted",
            Self::InviteDeclined { .. } => "invite_declined",
            Self::JoinedWorld { .. } => "joined_world",
            Self::Disconnected { .. } => "disconnected",
            Self::Reconnected => "reconnected",
            Self::Error { .. } => "error",
            Self::FriendOffline { .. } | Self::Presence { .. } | Self::Connected => return None,
        })
    }
}

/// Who may see what you're doing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    #[default]
    Everything,
    OnlineOnly,
    Invisible,
}

/// Where events go: the UI, then (once sound.rs has cues) a sound.
pub type Sink = Arc<dyn Fn(SocialEvent) + Send + Sync>;

/// One social backend. Calls block (run them off the UI thread); events arrive through the sink given to `connect`.
pub trait SocialProvider: Send {
    fn name(&self) -> &'static str;
    fn connect(&mut self, sink: Sink) -> Result<(), String>;
    fn disconnect(&mut self);
    fn friends(&self) -> Result<Vec<Friend>, String>;
    fn requests(&self) -> Result<Vec<Friend>, String>;
    fn request(&mut self, name: &str) -> Result<(), String>;
    fn answer_request(&mut self, id: &str, accept: bool) -> Result<(), String>;
    fn remove(&mut self, id: &str) -> Result<(), String>;
    fn conversations(&self) -> Result<Vec<Conversation>, String>;
    fn history(&self, conversation: &str, before: Option<&str>) -> Result<Vec<Message>, String>;
    fn send(&mut self, conversation: &str, text: &str) -> Result<Message, String>;
    fn invite(&mut self, friend: &str, pack: Option<PackRef>, target: &str) -> Result<Invite, String>;
    fn answer_invite(&mut self, id: &str, accept: bool) -> Result<(), String>;
    fn set_presence(&mut self, presence: Presence, privacy: Privacy) -> Result<(), String>;
}

/// The live provider (None: social is off and its UI hidden).
#[derive(Default, Clone)]
pub struct Social(Arc<Mutex<Option<Box<dyn SocialProvider>>>>);

impl Social {
    fn with<T>(&self, f: impl FnOnce(&mut dyn SocialProvider) -> Result<T, String>) -> Result<T, String> {
        let mut g = self.0.lock().map_err(|_| "social state is poisoned")?;
        let p = g.as_mut().ok_or("Social isn't turned on.")?;
        f(p.as_mut())
    }
}

fn sink(app: &tauri::AppHandle) -> Sink {
    let app = app.clone();
    Arc::new(move |e: SocialEvent| {
        if let Some(_cue) = e.cue() {
            // crate::sound::cue(_cue);  // wired once obelisk sound's cues land
        }
        let _ = app.emit("social-event", &e);
    })
}

/// Which provider is on: "off", "mock" (made-up friends and events, for building the UI), later "essential"/"obelisk".
#[tauri::command]
pub async fn social_use(app: tauri::AppHandle, social: tauri::State<'_, Social>, provider: String) -> Result<(), String> {
    let mut next: Option<Box<dyn SocialProvider>> = match provider.as_str() {
        "off" => None,
        "mock" => Some(Box::new(mock::Mock::default())),
        "essential" => Some(Box::new(essential::Essential::default())),
        other => return Err(format!("unknown social provider \"{other}\"")),
    };
    if let Some(p) = next.as_mut() { p.connect(sink(&app))? }
    let mut g = social.0.lock().map_err(|_| "social state is poisoned")?;
    if let Some(mut old) = g.take() { old.disconnect() }
    *g = next;
    Ok(())
}

#[tauri::command]
pub fn social_status(social: tauri::State<'_, Social>) -> Option<&'static str> {
    social.0.lock().ok()?.as_ref().map(|p| p.name())
}

#[tauri::command]
pub fn social_friends(social: tauri::State<'_, Social>) -> Result<serde_json::Value, String> {
    social.with(|p| Ok(serde_json::json!({ "friends": p.friends()?, "requests": p.requests()? })))
}

#[tauri::command]
pub fn social_conversations(social: tauri::State<'_, Social>) -> Result<Vec<Conversation>, String> {
    social.with(|p| p.conversations())
}

#[tauri::command]
pub fn social_history(social: tauri::State<'_, Social>, conversation: String, before: Option<String>) -> Result<Vec<Message>, String> {
    social.with(|p| p.history(&conversation, before.as_deref()))
}

#[tauri::command]
pub fn social_send(social: tauri::State<'_, Social>, conversation: String, text: String) -> Result<Message, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 2000 { return Err("A message is 1 to 2000 characters.".into()) }
    social.with(|p| p.send(&conversation, text))
}

#[tauri::command]
pub fn social_request(social: tauri::State<'_, Social>, name: String, id: Option<String>, accept: Option<bool>) -> Result<(), String> {
    social.with(|p| match (id, accept) {
        (Some(id), Some(a)) => p.answer_request(&id, a),
        (Some(id), None) => p.remove(&id),
        _ => p.request(name.trim()),
    })
}

#[tauri::command]
pub fn social_invite(social: tauri::State<'_, Social>, friend: String, pack: Option<PackRef>, target: String) -> Result<Invite, String> {
    social.with(|p| p.invite(&friend, pack, &target))
}

#[tauri::command]
pub fn social_answer_invite(social: tauri::State<'_, Social>, id: String, accept: bool) -> Result<(), String> {
    social.with(|p| p.answer_invite(&id, accept))
}

#[tauri::command]
pub fn social_presence(social: tauri::State<'_, Social>, presence: Presence, privacy: Privacy) -> Result<(), String> {
    social.with(|p| p.set_presence(presence, privacy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_and_cue() {
        let e = SocialEvent::Invite { invite: Invite { id: "i".into(), from: "f".into(), pack: None, target: "lan".into(), expires: None } };
        let back: SocialEvent = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
        assert_eq!(e.cue(), Some("invite"));
        assert_eq!(SocialEvent::Connected.cue(), None);
    }
}
