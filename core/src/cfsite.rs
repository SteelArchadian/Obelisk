//! The curseforge.com website's own data (what its pages load), for what the CurseForge API doesn't have: comments,
//! replies and the signed-in account's favorites. Signed-in calls carry the session cookies of the user's own sign-in
//! on curseforge.com (made in Obelisk's sign-in window; Obelisk never sees the password).

use serde_json::{json, Value};
use std::sync::Arc;

pub const SITE: &str = "https://www.curseforge.com";
const API: &str = "https://www.curseforge.com/api/v1";

fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    Ok(ureq::AgentBuilder::new().user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Obelisk")
        .timeout(std::time::Duration::from_secs(20)).tls_connector(Arc::new(tls)).build())
}

fn call(method: &str, path: &str, cookie: Option<&str>, body: Option<Value>) -> Result<(u16, Value), String> {
    let mut req = agent()?.request(method, &format!("{API}{path}")).set("Accept", "application/json")
        .set("Origin", SITE).set("Referer", &format!("{SITE}/"));
    if let Some(c) = cookie.filter(|c| !c.is_empty()) {
        req = req.set("Cookie", c);
    }
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match res {
        Ok(r) => {
            let code = r.status();
            let text = r.into_string().unwrap_or_default();
            Ok((code, serde_json::from_str(&text).unwrap_or(Value::Null)))
        }
        Err(ureq::Error::Status(code, r)) => Ok((code, serde_json::from_str(&r.into_string().unwrap_or_default()).unwrap_or(Value::Null))),
        Err(e) => Err(format!("couldn't reach curseforge.com: {e}")),
    }
}

/// The signed-in account ({id, username, displayName, avatarUrl}), or None when the cookies hold no session.
pub fn profile(cookie: &str) -> Result<Option<Value>, String> {
    let (code, v) = call("GET", "/users/profile", Some(cookie), None)?;
    if code == 401 || code == 403 {
        return Ok(None);
    }
    if code >= 300 {
        return Err(format!("curseforge.com answered {code}"));
    }
    let d = if v["data"].is_object() { &v["data"] } else { &v };
    let id = d["id"].as_u64().or_else(|| d["userId"].as_u64());
    Ok(id.map(|id| json!({
        "id": id,
        "username": d["username"].as_str().or(d["userName"].as_str()).unwrap_or_default(),
        "displayName": d["displayName"].as_str().or(d["username"].as_str()).unwrap_or_default(),
        "avatarUrl": d["avatarUrl"].as_str().or(d["twitchAvatarUrl"].as_str()).or(d["avatar"].as_str()).unwrap_or_default(),
    })))
}

/// An author's picture: Twitch ones leave their size as "{0}" for the page to fill in; none at all is None.
fn avatar(url: Option<&str>) -> Option<String> {
    url.map(str::trim).filter(|u| u.starts_with("http")).map(|u| u.replace("{0}", "70x70"))
}

/// One page (20, newest first) of a project's comments, replies nested under each: {comments, total}.
pub fn comments(project_id: u64, page: u32, cookie: Option<&str>) -> Result<Value, String> {
    let (code, v) = call("GET", &format!("/mods/{project_id}/comments?page={page}&size=20"), cookie, None)?;
    if code >= 300 {
        return Err(format!("couldn't load the comments (curseforge.com answered {code})"));
    }
    let map = |c: &Value| json!({
        "id": c["id"], "html": c["renderedHtml"].as_str().or(c["body"].as_str()).unwrap_or_default(),
        "posted": c["datePosted"], "pinned": c["isPinned"], "parentId": c["parentId"],
        "author": { "id": c["author"]["id"], "username": c["author"]["username"], "name": c["author"]["displayName"],
            "avatar": avatar(c["author"]["twitchAvatarUrl"].as_str()) },
    });
    let list: Vec<Value> = v["data"].as_array().into_iter().flatten().map(|c| {
        let mut top = map(c);
        top["replies"] = c["replies"].as_array().into_iter().flatten().map(map).collect::<Vec<_>>().into();
        top
    }).collect();
    Ok(json!({ "comments": list, "total": v["pagination"]["totalCount"] }))
}

/// Text typed in Obelisk as the HTML the site stores: paragraphs, with everything escaped.
pub fn text_html(text: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    text.trim().split("\n\n").filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", esc(p.trim()).replace('\n', "<br>"))).collect()
}

/// Post a comment on a project, or a reply under comment `parent`.
pub fn post_comment(cookie: &str, project_id: u64, text: &str, parent: Option<u64>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("write something first".into());
    }
    let body = json!({ "entityId": project_id, "body": text_html(text), "parentId": parent, "bodyType": "RawHtml" });
    match call("POST", "/comments", Some(cookie), Some(body))?.0 {
        200..=299 => Ok(()),
        401 | 403 => Err("CurseForge says you're not signed in; sign in again".into()),
        429 => Err("CurseForge says you're commenting too fast; wait a minute".into()),
        c => Err(format!("CurseForge didn't take the comment (it answered {c})")),
    }
}

/// The ids of the account's favorite projects.
pub fn favorites(cookie: &str) -> Result<Vec<u64>, String> {
    let (code, v) = call("GET", "/users/favorites", Some(cookie), None)?;
    if code >= 300 {
        return Err(format!("couldn't load your favorites (curseforge.com answered {code})"));
    }
    let list = if v["data"].is_array() { &v["data"] } else { &v };
    Ok(list.as_array().into_iter().flatten()
        .filter_map(|f| f.as_u64().or(f["id"].as_u64()).or(f["projectId"].as_u64()).or(f["modId"].as_u64())).collect())
}

/// Add a project to the account's favorites, or take it off.
pub fn set_favorite(cookie: &str, project_id: u64, on: bool) -> Result<(), String> {
    match call("POST", &format!("/users/{project_id}/favorite"), Some(cookie), Some(json!({ "isFavorite": on })))?.0 {
        200..=299 => Ok(()),
        401 | 403 => Err("CurseForge says you're not signed in; sign in again".into()),
        c => Err(format!("CurseForge didn't change the favorite (it answered {c})")),
    }
}

/// The account's notifications, newest first, as the site's bell and legacy.curseforge.com/my-notifications show
/// them: approvals and status changes of the user's projects and files, comments on them, replies, messages, invites.
/// Each is {id: "site-N", siteId, kind, title, text, label, projectId, tab, url, read, created}.
pub fn notifications(cookie: &str, size: u32) -> Result<Vec<Value>, String> {
    let (code, v) = call("GET", &format!("/notifications?index=0&pageSize={size}"), Some(cookie), None)?;
    if code == 401 || code == 403 {
        return Err("CurseForge says you're not signed in; sign in again".into());
    }
    if code >= 300 {
        return Err(format!("couldn't load your CurseForge notifications (curseforge.com answered {code})"));
    }
    Ok(v["data"].as_array().into_iter().flatten().filter(|n| n["status"].as_u64() != Some(1)).map(note).collect())
}

/// How many notifications the site counts as new (its bell's badge).
pub fn notifications_count(cookie: &str) -> Result<u64, String> {
    let (code, v) = call("GET", "/notifications/count", Some(cookie), None)?;
    if code >= 300 {
        return Err(format!("curseforge.com answered {code}"));
    }
    Ok(v.as_u64().or(v["data"].as_u64()).or(v["count"].as_u64()).unwrap_or(0))
}

/// Mark notifications read on the site (all of them when `ids` is None).
pub fn notifications_read(cookie: &str, ids: Option<Vec<u64>>) -> Result<(), String> {
    let body = match ids {
        Some(ids) if ids.is_empty() => return Ok(()),
        Some(ids) => json!({ "ids": ids, "status": 3 }),
        None => json!({ "status": 3 }),
    };
    match call("PUT", "/notifications/mark-status", Some(cookie), Some(body))?.0 {
        200..=299 => Ok(()),
        c => Err(format!("CurseForge didn't mark them read (it answered {c})")),
    }
}

/// One site notification in the bell's words (the site's own English texts).
fn note(n: &Value) -> Value {
    let m = &n["metadata"];
    let s = |k: &str| m[k].as_str().unwrap_or_default().to_string();
    let project = if s("projectName").is_empty() { "your project".to_string() } else { s("projectName") };
    let file = s("fileName");
    let page = (!s("projectSlug").is_empty()).then(|| format!("{SITE}/{}/{}/{}",
        if s("gameSlug").is_empty() { "minecraft".into() } else { s("gameSlug") },
        if s("classSlug").is_empty() { "mc-mods".into() } else { s("classSlug") }, s("projectSlug")));
    let (kind, title, label, tab, url) = match n["notificationType"].as_u64().unwrap_or(0) {
        1 => {
            let st = m["newProjectStatus"].as_u64().unwrap_or(0);
            let title = match st {
                4 => format!("\"{project}\" has been approved by moderation and is now live!"),
                2 => format!("\"{project}\" requires changes before approval"),
                5 => format!("\"{project}\" was rejected by moderation"),
                _ => format!("The status of the project \"{project}\" was changed"),
            };
            let label = ["", "New", "Changes Required", "Under Soft Review", "Approved", "Rejected", "Changes Made", "Inactive",
                "Abandoned", "Deleted", "Under Review"].get(st as usize).copied().unwrap_or_default();
            ("status", title, label, None, page.clone())
        }
        2 => (
            "comment",
            format!("{} made a comment on \"{project}\"", if s("authorName").is_empty() { "Someone".into() } else { s("authorName") }),
            "", Some("comments"), page.as_ref().map(|p| format!("{p}/comments")),
        ),
        3 => {
            let st = m["newProjectFileStatus"].as_u64().unwrap_or(0);
            let title = match st {
                4 => format!("{file} has been approved by moderation and is now live!"),
                2 => format!("{file} requires changes before approval"),
                5 => format!("{file} was rejected by moderation"),
                _ => format!("The status of the file {file} was changed"),
            };
            let label = ["", "Processing", "Changes Required", "Under Review", "Approved", "Rejected", "Malware Detected", "Deleted",
                "Archived", "Testing", "Released", "Ready for Review", "Deprecated", "Baking", "Awaiting Publishing",
                "Failed Publishing", "Cooking", "Cooked", "Under Manual Review", "Scanning for Malware", "Processing File",
                "Pending Release"].get(st as usize).copied().unwrap_or_default();
            ("file", title, label, Some("versions"), page.clone())
        }
        4 => (
            "message",
            format!("New private message from {} about \"{}\"", if s("authorName").is_empty() { "someone".into() } else { s("authorName") }, s("subjectTitle")),
            "", None, m["conversationId"].as_u64().map(|c| format!("https://legacy.curseforge.com/private-messages/{c}")),
        ),
        5 => ("invite", format!("You were invited to join \"{project}\""), "", None, page.clone()),
        6 => ("invite", format!("{} answered your invitation to join \"{project}\"", s("displayName")), "", None, page.clone()),
        7 => ("invite", format!("You were added to the project \"{project}\""), "", None, page.clone()),
        8 => ("file", format!("The project {project} has a new file update: {file}"), "", Some("versions"), page.clone()),
        9 => (
            "reply",
            format!("{} replied to your comment on \"{project}\"", if s("authorName").is_empty() { "Someone".into() } else { s("authorName") }),
            "", Some("comments"), page.as_ref().map(|p| format!("{p}/comments")),
        ),
        _ => ("other", format!("Notification about \"{project}\""), "", None, page.clone()),
    };
    // What was said: a comment's text, or the note moderation left with a status change.
    let text = ["textContent", "notes", "note", "reason", "message", "comment"].iter()
        .find_map(|k| m[*k].as_str().filter(|t| !t.trim().is_empty())).unwrap_or_default();
    json!({
        "id": format!("site-{}", n["id"]), "siteId": n["id"], "kind": kind, "title": title, "text": text, "label": label,
        "projectId": m["projectId"], "tab": tab, "url": url,
        "read": n["status"].as_u64() == Some(3), "created": n["dateCreated"],
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn site_notifications_read_like_the_site() {
        let n = super::note(&serde_json::json!({ "id": 7, "notificationType": 3, "status": 5, "dateCreated": "2026-10-30T10:00:00Z",
            "metadata": { "projectId": 12, "projectName": "Kaleidoscope Compat", "fileName": "kc-1.2.jar", "newProjectFileStatus": 4,
                "gameSlug": "minecraft", "classSlug": "mc-mods", "projectSlug": "kaleidoscope-compat" } }));
        assert_eq!(n["title"], "kc-1.2.jar has been approved by moderation and is now live!");
        assert_eq!(n["label"], "Approved");
        assert_eq!(n["read"], false);
        assert_eq!(n["url"], "https://www.curseforge.com/minecraft/mc-mods/kaleidoscope-compat");
        let c = super::note(&serde_json::json!({ "id": 8, "notificationType": 2, "status": 3,
            "metadata": { "projectId": 12, "projectName": "KC", "authorName": "Alex", "textContent": "nice" } }));
        assert_eq!(c["title"], "Alex made a comment on \"KC\"");
        assert_eq!((c["text"].as_str(), c["tab"].as_str(), c["read"].as_bool()), (Some("nice"), Some("comments"), Some(true)));
    }

    #[test]
    fn typed_text_becomes_escaped_paragraphs() {
        assert_eq!(super::avatar(Some("https://x/a-profile_image-{0}.png")).as_deref(), Some("https://x/a-profile_image-70x70.png"));
        assert_eq!(super::avatar(Some("")), None);
        assert_eq!(super::text_html("hi <b>\nthere\n\nnext & last"), "<p>hi &lt;b&gt;<br>there</p><p>next &amp; last</p>");
    }
}
