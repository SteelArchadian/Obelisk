//! Essential's social system (friends, messages, world invites), for if the collaboration happens. Essential is closed
//! source and has no public API for this yet: whatever they share (an HTTP/WebSocket API, a client library, or a bridge
//! through the Essential mod running in the game) gets mapped onto `SocialProvider` here. Until then it says so.

use super::*;

#[derive(Default)]
pub struct Essential;

const NOT_YET: &str = "Essential's social system isn't connected to Obelisk yet.";

impl SocialProvider for Essential {
    fn name(&self) -> &'static str { "essential" }
    fn connect(&mut self, _sink: Sink) -> Result<(), String> { Err(NOT_YET.into()) }
    fn disconnect(&mut self) {}
    fn friends(&self) -> Result<Vec<Friend>, String> { Err(NOT_YET.into()) }
    fn requests(&self) -> Result<Vec<Friend>, String> { Err(NOT_YET.into()) }
    fn request(&mut self, _name: &str) -> Result<(), String> { Err(NOT_YET.into()) }
    fn answer_request(&mut self, _id: &str, _accept: bool) -> Result<(), String> { Err(NOT_YET.into()) }
    fn remove(&mut self, _id: &str) -> Result<(), String> { Err(NOT_YET.into()) }
    fn conversations(&self) -> Result<Vec<Conversation>, String> { Err(NOT_YET.into()) }
    fn history(&self, _c: &str, _before: Option<&str>) -> Result<Vec<Message>, String> { Err(NOT_YET.into()) }
    fn send(&mut self, _c: &str, _text: &str) -> Result<Message, String> { Err(NOT_YET.into()) }
    fn invite(&mut self, _f: &str, _pack: Option<PackRef>, _target: &str) -> Result<Invite, String> { Err(NOT_YET.into()) }
    fn answer_invite(&mut self, _id: &str, _accept: bool) -> Result<(), String> { Err(NOT_YET.into()) }
    fn set_presence(&mut self, _p: Presence, _privacy: Privacy) -> Result<(), String> { Err(NOT_YET.into()) }
}
