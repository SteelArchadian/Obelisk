//! Which CurseForge endpoints accept the saved key (key never printed).
fn main() {
    let key = keyring::Entry::new("Obelisk", "curseforge-api-key").unwrap().get_password().expect("no key saved");
    let a = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(native_tls::TlsConnector::new().unwrap())).build();
    let code = |r: Result<ureq::Response, ureq::Error>| match r { Ok(r) => r.status(), Err(ureq::Error::Status(c, _)) => c, Err(_) => 0 };
    let get = |p: &str| code(a.get(&format!("https://api.curseforge.com{p}")).set("x-api-key", &key).call());
    let post = |p: &str, b: serde_json::Value| code(a.post(&format!("https://api.curseforge.com{p}")).set("x-api-key", &key).send_json(b));
    println!("categories   {}", get("/v1/categories?gameId=432"));
    println!("versions     {}", get("/v1/minecraft/version"));
    println!("mods batch   {}", post("/v1/mods", serde_json::json!({"modIds":[238222]})));
    println!("fingerprints {}", post("/v1/fingerprints/432", serde_json::json!({"fingerprints":[1]})));
    println!("featured     {}", post("/v1/mods/featured", serde_json::json!({"gameId":432,"excludedModIds":[],"gameVersionTypeId":null})));
    println!("files batch  {}", post("/v1/mods/files", serde_json::json!({"fileIds":[3000000]})));
    println!("search       {}", get("/v1/mods/search?gameId=432&pageSize=1"));
}
