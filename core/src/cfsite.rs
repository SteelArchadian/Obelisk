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
            "avatar": c["author"]["twitchAvatarUrl"] },
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

#[cfg(test)]
mod tests {
    #[test]
    fn typed_text_becomes_escaped_paragraphs() {
        assert_eq!(super::text_html("hi <b>\nthere\n\nnext & last"), "<p>hi &lt;b&gt;<br>there</p><p>next &amp; last</p>");
    }
}
