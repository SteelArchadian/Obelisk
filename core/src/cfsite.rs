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
        "read": n["status"].as_u64() == Some(3), "created": n["dateCreated"].as_str().and_then(iso_ms),
        "key": if file.is_empty() { s("projectName") } else { file.clone() },
    })
}

/// A row of legacy.curseforge.com/my-notifications (as the reading script hands it over: its date, its lines of text,
/// whether it's unread, its links) as a bell notification.
pub fn legacy_item(i: &Value, read: &[String]) -> Option<Value> {
    let lines: Vec<&str> = i["lines"].as_array()?.iter().filter_map(Value::as_str).collect();
    let title = lines.first()?.to_string();
    let text = lines[1..].join(" ");
    let epoch = i["epoch"].as_u64().unwrap_or(0);
    let id = format!("legacy-{epoch}-{:x}", title.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)));
    let links: Vec<&str> = i["links"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    // The project it's about: a /projects/<id> link, or a /<game>/<class>/<slug> one (not a member's page).
    let project_id = links.iter().find_map(|l| l.split("/projects/").nth(1)?.split(['/', '?', '#']).next()?.parse::<u64>().ok());
    let slug = links.iter().find_map(|l| {
        let path = l.split("curseforge.com/").nth(1)?;
        let parts: Vec<&str> = path.split(['?', '#']).next()?.split('/').collect();
        (parts.len() >= 3 && parts[0] != "members" && parts[0] != "projects" && !parts[2].is_empty()).then(|| parts[2].to_string())
    });
    let lower = format!("{title} {text}").to_lowercase();
    let (kind, tab) = if lower.contains("comment") { ("comment", Some("comments")) }
        else if lower.contains("file") { ("file", Some("versions")) }
        else { ("status", None) };
    let label = ["approved", "rejected", "changes required", "under review", "deleted", "inactive", "abandoned"].iter()
        .find(|w| lower.contains(*w)).map(|w| w.split(' ').map(|p| { let mut c = p.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default() }).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    Some(json!({
        "id": id, "kind": kind, "title": title, "text": text, "label": label, "projectId": project_id, "slug": slug, "tab": tab,
        "url": links.first().map(|l| l.to_string()).unwrap_or_else(|| "https://legacy.curseforge.com/my-notifications".to_string()),
        "read": !i["unread"].as_bool().unwrap_or(false) || read.contains(&id), "created": epoch * 1000, "legacy": true,
    }))
}

/// %XX escapes back to text (what encodeURIComponent made).
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// "2026-10-23T18:24:08.327Z" as milliseconds since 1970 (UTC).
pub fn iso_ms(s: &str) -> Option<u64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let ms = if s.get(19..20) == Some(".") { s[20..].chars().take(3).collect::<String>().parse::<i64>().unwrap_or(0) } else { 0 };
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    u64::try_from(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + ms).ok()
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

    #[test]
    fn a_page_row_becomes_a_notification() {
        assert_eq!(super::decode("%7B%22a%22%3A%22%C3%A9%20x%22%7D"), "{\"a\":\"é x\"}");
        let n = super::legacy_item(&serde_json::json!({ "epoch": 1790000000u64, "unread": true,
            "lines": ["The file kc-1.2.jar for Kaleidoscope Compat has been approved", "Looks good."],
            "links": ["https://legacy.curseforge.com/minecraft/mc-mods/kaleidoscope-compat/files/123"] }), &[]).unwrap();
        assert_eq!((n["kind"].as_str(), n["label"].as_str(), n["slug"].as_str()), (Some("file"), Some("Approved"), Some("kaleidoscope-compat")));
        assert_eq!((n["text"].as_str(), n["read"].as_bool(), n["created"].as_u64()), (Some("Looks good."), Some(false), Some(1790000000000)));
    }

    #[test]
    fn site_dates_become_milliseconds() {
        assert_eq!(super::iso_ms("2026-10-23T18:24:08.327Z"), Some(1792779848327));
        assert_eq!(super::iso_ms("1970-01-02T00:00:00Z"), Some(86_400_000));
    }
}
