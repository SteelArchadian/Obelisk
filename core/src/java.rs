//! Java installs on this PC, for picking one per profile.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JavaInstall {
    /// The javaw.exe Prism should run.
    pub path: String,
    /// Full version ("21.0.7") and its major number (21; 8 for "1.8.0_481").
    pub version: String,
    pub major: u32,
    pub vendor: String,
}

/// Major Java number from a version string: "1.8.0_481" -> 8, "21.0.7" -> 21.
pub fn major_of(version: &str) -> u32 {
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
    match parts.next().and_then(|s| s.parse().ok()) {
        Some(1) => parts.next().and_then(|s| s.parse().ok()).unwrap_or(1),
        Some(n) => n,
        None => 0,
    }
}

/// A Java home's install, read from its `release` file.
fn read_home(home: &Path) -> Option<JavaInstall> {
    let exe = ["javaw.exe", "java.exe", "java"].iter().map(|n| home.join("bin").join(n)).find(|p| p.is_file())?;
    let text = fs::read_to_string(home.join("release")).unwrap_or_default();
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string());
    let version = field("JAVA_VERSION")?;
    let vendor = field("IMPLEMENTOR").unwrap_or_else(|| {
        let name = home.to_string_lossy().to_lowercase();
        [("adoptium", "Eclipse Adoptium"), ("microsoft", "Microsoft"), ("zulu", "Azul Zulu"), ("corretto", "Amazon Corretto"),
         ("bellsoft", "BellSoft"), ("graalvm", "GraalVM"), ("java-runtime", "Mojang"), ("\\java\\", "Oracle")]
            .iter().find(|(k, _)| name.contains(k)).map(|(_, v)| v.to_string()).unwrap_or_default()
    });
    Some(JavaInstall { path: exe.to_string_lossy().into_owned(), major: major_of(&version), version, vendor })
}

/// Folders whose subfolders are Java homes: Prism's own downloads, then the usual vendor folders.
fn roots(data: &Path) -> Vec<PathBuf> {
    let mut v = vec![data.join("java")];
    for env in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(env).map(PathBuf::from) else { continue };
        let base = if env == "LOCALAPPDATA" { base.join("Programs") } else { base };
        for vendor in ["Java", "Eclipse Adoptium", "Microsoft", "Zulu", "BellSoft", "Amazon Corretto", "GraalVM", "Semeru"] {
            v.push(base.join(vendor));
        }
    }
    v
}

/// Every Java install found, newest major first; duplicates (same javaw) dropped.
pub fn installs(data: &Path) -> Vec<JavaInstall> {
    let mut homes: Vec<PathBuf> = roots(data).iter()
        .filter_map(|r| fs::read_dir(r).ok())
        .flat_map(|d| d.flatten().map(|e| e.path()))
        .collect();
    if let Some(h) = std::env::var_os("JAVA_HOME") { homes.push(h.into()); }
    let mut out: Vec<JavaInstall> = Vec::new();
    for home in homes {
        // Mojang runtimes nest one level deeper (java-runtime-x/windows-x64/java-runtime-x).
        let found = read_home(&home).or_else(|| fs::read_dir(&home).ok()?.flatten()
            .find_map(|e| read_home(&e.path()).or_else(|| fs::read_dir(e.path()).ok()?.flatten().find_map(|f| read_home(&f.path())))));
        if let Some(j) = found {
            let canon = |p: &str| fs::canonicalize(p).unwrap_or_else(|_| p.into());
            if !out.iter().any(|o| canon(&o.path) == canon(&j.path)) { out.push(j); }
        }
    }
    out.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| b.version.cmp(&a.version)));
    out
}

/// The install a java/javaw executable (or a Java home folder) belongs to, wherever it is.
pub fn at(path: &Path) -> Option<JavaInstall> {
    let home = if path.is_file() { path.parent()?.parent()? } else { path };
    read_home(home)
}

/// Where `install` puts Eclipse Temurin `major`: Prism's own java folder, which `installs` scans.
pub fn download_dir(data: &Path, major: u32) -> PathBuf {
    data.join("java").join(format!("temurin-{major}-jre"))
}

/// Download the newest Eclipse Temurin JRE `major` from Adoptium and unpack it into `dest`.
/// `progress(done, total)` is called as the download goes. Done in-process on purpose: a hidden
/// script that downloads and unpacks an executable is what antivirus heuristics flag.
pub fn install(major: u32, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<JavaInstall, String> {
    use std::io::Read;
    let url = format!("https://api.adoptium.net/v3/binary/latest/{major}/ga/windows/x64/jre/hotspot/normal/eclipse");
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).build();
    let resp = agent.get(&url).call().map_err(|e| format!("download: {e}"))?;
    let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(total as usize);
    let mut reader = resp.into_reader().take(1 << 30);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download: {e}"))?;
        if n == 0 { break }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64, total);
    }
    let parent = dest.parent().ok_or("bad folder")?;
    let tmp = parent.join(format!(".unpacking-{major}"));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip: {e}"))?;
    zip.extract(&tmp).map_err(|e| format!("unpacking: {e}"))?;
    // The zip holds one folder (jdk-25.0.4.1+1-jre): that becomes `dest`.
    let inner = fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.is_dir()).ok_or("empty download")?;
    let _ = fs::remove_dir_all(dest);
    fs::rename(&inner, dest).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&tmp);
    at(dest).ok_or_else(|| format!("Java {major} downloaded but isn't usable"))
}

/// Whether a Java major suits a Minecraft that needs `need` (old Forge breaks past Java 8).
pub fn suits(major: u32, need: u32) -> bool {
    if need == 8 { major == 8 } else { major >= need }
}

/// The install to use for `need`: the exact major if present, else the closest newer one that suits.
pub fn best(list: &[JavaInstall], need: u32) -> Option<&JavaInstall> {
    list.iter().filter(|j| suits(j.major, need)).min_by_key(|j| (j.major != need, j.major))
}

/// Before launch: if the profile pins a Java that is missing or wrong for its Minecraft, switch it to a fitting
/// install, or hand the choice back to Prism when none is installed. Returns what changed, for the user.
pub fn check_before_launch(data: &Path, id: &str) -> Result<Option<String>, String> {
    let root = crate::instance_root(data, id)?;
    let cfg = crate::read_cfg(&root.join("instance.cfg"));
    if cfg.get("OverrideJavaLocation").map(String::as_str) != Some("true") { return Ok(None); }
    let Some(inst) = crate::list_instances(data).into_iter().find(|i| i.id == id) else { return Ok(None) };
    let need = crate::sides::java_for(&inst.minecraft);
    let list = installs(data);
    let path = cfg.get("JavaPath").cloned().unwrap_or_default();
    let canon = |p: &str| fs::canonicalize(p).unwrap_or_else(|_| p.into());
    // A Java outside the scanned folders (picked with Other Java) still counts.
    let outside = list.iter().all(|j| canon(&j.path) != canon(&path)).then(|| at(Path::new(&path))).flatten();
    let current = list.iter().find(|j| canon(&j.path) == canon(&path)).or(outside.as_ref());
    if let Some(j) = current.filter(|j| suits(j.major, need)) {
        // Prism only takes the Java versions its own list names for this Minecraft (1.21.1: just 21) and quietly
        // refuses to start with any other. The player picked this newer one on purpose, and it runs this
        // Minecraft, so Prism is told to accept it.
        let listed = prism_java_majors(data, &inst.minecraft);
        let ignored = cfg.get("IgnoreJavaCompatibility").is_some_and(|v| v == "true");
        if !listed.is_empty() && !listed.contains(&j.major) && !ignored {
            crate::update_cfg(&root.join("instance.cfg"), &[("IgnoreJavaCompatibility", Some("true".into()))])?;
            return Ok(Some(format!("This profile uses Java {}, which Prism doesn't list for Minecraft {}; Prism now accepts it.", j.major, inst.minecraft)));
        }
        return Ok(None);
    }
    let why = match current {
        Some(j) => format!("Java {} doesn't suit Minecraft {} (it needs Java {need})", j.major, inst.minecraft),
        None => "The chosen Java is no longer installed".to_string(),
    };
    let cfg_path = root.join("instance.cfg");
    match best(&list, need) {
        Some(j) => {
            crate::update_cfg(&cfg_path, &[("JavaPath", Some(j.path.clone()))])?;
            Ok(Some(format!("{why}, so this profile now uses Java {} ({}).", j.major, j.version)))
        }
        None => {
            crate::update_cfg(&cfg_path, &[("OverrideJavaLocation", Some("false".into()))])?;
            Ok(Some(format!("{why}, and none is installed, so Prism will pick or download one.")))
        }
    }
}

/// The Java majors Prism's metadata lists for a Minecraft version (empty when it hasn't fetched them).
pub fn prism_java_majors(data: &Path, mc: &str) -> Vec<u32> {
    if mc.is_empty() || mc.contains(['/', '\\']) || mc.contains("..") { return vec![] }
    fs::read_to_string(data.join("meta").join("net.minecraft").join(format!("{mc}.json"))).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["compatibleJavaMajors"].as_array().map(|a| a.iter().filter_map(|m| m.as_u64().map(|m| m as u32)).collect()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majors() {
        assert_eq!(major_of("1.8.0_481"), 8);
        assert_eq!(major_of("21.0.7"), 21);
        assert_eq!(major_of("17"), 17);
        assert_eq!(major_of(""), 0);
    }

    #[test]
    fn picks() {
        let j = |major, v: &str| JavaInstall { path: v.into(), version: v.into(), major, vendor: String::new() };
        let list = [j(25, "25"), j(21, "21"), j(8, "1.8")];
        assert_eq!(best(&list, 21).unwrap().major, 21);
        assert_eq!(best(&list, 17).unwrap().major, 21);
        assert_eq!(best(&list, 8).unwrap().major, 8);
        assert!(best(&list[..2], 8).is_none());
    }

    #[test]
    #[ignore] // reads this PC's folders: cargo test java -- --ignored --nocapture
    fn list_here() {
        for j in installs(&crate::data_dir()) { println!("{j:?}"); }
    }
}
