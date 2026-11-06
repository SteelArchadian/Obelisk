//! Made-up friends and a little scripted traffic, so the friends panel, chat, invite card and sound cues can be built
//! and tried before a real provider exists. Nothing leaves the PC.

use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

fn friend(id: &str, name: &str, presence: Presence) -> Friend {
    Friend { id: id.into(), name: name.into(), uuid: None, avatar: None, presence, since: Some(now() - 86_400_000 * 30) }
}

#[derive(Default)]
pub struct Mock {
    friends: Vec<Friend>,
    messages: Vec<Message>,
    sink: Option<Sink>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl SocialProvider for Mock {
    fn name(&self) -> &'static str { "mock" }

    fn connect(&mut self, sink: Sink) -> Result<(), String> {
        let pack = PackRef { name: "Test Dummy".into(), mc: "1.21.1".into(), loader: "neoforge".into(), mods_hash: None, source: None };
        self.friends = vec![
            friend("m1", "Pebble", Presence::Playing { pack: Some(pack.clone()), world: Some("Cozy Base".into()), joinable: true }),
            friend("m2", "Mossy", Presence::Online),
            friend("m3", "Quartz", Presence::Offline),
        ];
        sink(SocialEvent::Connected);
        // A short script, a few seconds apart: someone comes online, writes, then invites you.
        let (s, stop) = (sink.clone(), self.stop.clone());
        std::thread::spawn(move || {
            let msg = |text: &str| Message { id: format!("x{}", now()), conversation: "m2".into(), from: "m2".into(), text: text.into(), sent: now() };
            let script: Vec<SocialEvent> = vec![
                SocialEvent::FriendOnline { id: "m3".into() },
                SocialEvent::Message { message: msg("hey! you on?") },
                SocialEvent::Invite { invite: Invite { id: "inv1".into(), from: "m1".into(), pack: Some(pack), target: "lan:Cozy Base".into(), expires: Some(now() + 300_000) } },
            ];
            for e in script {
                std::thread::sleep(Duration::from_secs(4));
                if stop.load(std::sync::atomic::Ordering::Relaxed) { return }
                s(e);
            }
        });
        self.sink = Some(sink);
        Ok(())
    }

    fn disconnect(&mut self) { self.stop.store(true, std::sync::atomic::Ordering::Relaxed); }
    fn friends(&self) -> Result<Vec<Friend>, String> { Ok(self.friends.clone()) }
    fn requests(&self) -> Result<Vec<Friend>, String> { Ok(vec![friend("m4", "Clover", Presence::Online)]) }
    fn request(&mut self, _name: &str) -> Result<(), String> { Ok(()) }

    fn answer_request(&mut self, id: &str, accept: bool) -> Result<(), String> {
        if accept {
            let f = friend(id, "Clover", Presence::Online);
            self.friends.push(f.clone());
            if let Some(s) = &self.sink { s(SocialEvent::FriendAccepted { friend: f }) }
        }
        Ok(())
    }

    fn remove(&mut self, id: &str) -> Result<(), String> { self.friends.retain(|f| f.id != id); Ok(()) }

    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        Ok(self.friends.iter().map(|f| Conversation {
            id: f.id.clone(), title: f.name.clone(), members: vec![f.id.clone()], unread: 0,
            last: self.messages.iter().rev().find(|m| m.conversation == f.id).cloned(),
        }).collect())
    }

    fn history(&self, conversation: &str, _before: Option<&str>) -> Result<Vec<Message>, String> {
        Ok(self.messages.iter().filter(|m| m.conversation == conversation).cloned().collect())
    }

    fn send(&mut self, conversation: &str, text: &str) -> Result<Message, String> {
        let m = Message { id: format!("s{}", now()), conversation: conversation.into(), from: "me".into(), text: text.into(), sent: now() };
        self.messages.push(m.clone());
        if let Some(s) = &self.sink { s(SocialEvent::MessageSent { message: m.clone() }) }
        Ok(m)
    }

    fn invite(&mut self, friend: &str, pack: Option<PackRef>, target: &str) -> Result<Invite, String> {
        let inv = Invite { id: format!("o{}", now()), from: "me".into(), pack, target: target.into(), expires: Some(now() + 300_000) };
        if let Some(s) = &self.sink { s(SocialEvent::InviteAccepted { invite: inv.id.clone(), by: friend.into() }) }
        Ok(inv)
    }

    fn answer_invite(&mut self, _id: &str, _accept: bool) -> Result<(), String> { Ok(()) }
    fn set_presence(&mut self, _p: Presence, _privacy: Privacy) -> Result<(), String> { Ok(()) }
}
