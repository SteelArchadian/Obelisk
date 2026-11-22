//! First run on a PC without Prism: fetch Prism Launcher's own release from its GitHub page and unpack it to the
//! per-user folder (where Prism's own installer puts it), so a new user needs no second download (spec 11-09).
//! Unpacked in-process like Java (no hidden installer run, which is what antivirus heuristics flag).

use std::fs;
use std::io::Read;
use std::path::Path;

const API: &str = "https://api.github.com/repos/PrismLauncher/PrismLauncher/releases/latest";

/// The Windows x64 non-portable zip of the newest Prism release: (version, url, sha256 when GitHub lists one).
/// MinGW build on purpose: it carries its own runtime, the MSVC one can need the VC++ redistributable.
pub fn pick(release: &serde_json::Value) -> Option<(String, String, Option<String>)> {
    let tag = release["tag_name"].as_str()?.to_string();
    let want = format!("PrismLauncher-Windows-MinGW-w64-{tag}.zip");
    let a = release["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(want.as_str()))?;
    let sha = a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase);
    Some((tag, a["browser_download_url"].as_str()?.to_string(), sha))
}

/// Download and unpack Prism into `dest` (the folder that will hold prismlauncher.exe). Returns the version.
pub fn install(dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<String, String> {
    use sha2::Digest;
    if dest.join("prismlauncher.exe").is_file() { return Err("Prism is already installed".into()) }
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).user_agent("Obelisk").build();
    let rel: serde_json::Value = agent.get(API).call().map_err(|e| format!("Prism's release page: {e}"))?
        .into_json().map_err(|e| format!("Prism's release page: {e}"))?;
    let (ver, url, sha) = pick(&rel).ok_or("Prism's newest release has no Windows download")?;
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
    if let Some(want) = sha {
        let got = format!("{:x}", sha2::Sha256::digest(&bytes));
        if got != want { return Err("the Prism download was damaged (checksum mismatch); try again".into()) }
    }
    let parent = dest.parent().ok_or("bad folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(".obelisk-prism-unpacking");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip: {e}"))?;
    zip.extract(&tmp).map_err(|e| format!("unpacking: {e}"))?;
    // The exe sits at the zip's root or inside one top folder; that level becomes `dest`.
    let root = if tmp.join("prismlauncher.exe").is_file() { tmp.clone() } else {
        fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path())
            .find(|p| p.join("prismlauncher.exe").is_file()).ok_or("the Prism download has no prismlauncher.exe")?
    };
    // A portable marker would make Prism keep its data beside the exe instead of %APPDATA%\PrismLauncher.
    let _ = fs::remove_file(root.join("portable.txt"));
    if dest.exists() { fs::remove_dir(dest).map_err(|_| format!("{} already exists and isn't empty", dest.display()))? }
    fs::rename(&root, dest).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&tmp);
    Ok(ver)
}

#[cfg(test)]
mod tests {
    #[test]
    fn picks_mingw_x64_zip() {
        let rel = serde_json::json!({"tag_name": "11.1.1", "assets": [
            {"name": "PrismLauncher-Windows-MinGW-arm64-11.1.1.zip", "browser_download_url": "a"},
            {"name": "PrismLauncher-Windows-MinGW-w64-Portable-11.1.1.zip", "browser_download_url": "b"},
            {"name": "PrismLauncher-Windows-MinGW-w64-11.1.1.zip", "browser_download_url": "c", "digest": "sha256:AB"},
        ]});
        assert_eq!(super::pick(&rel), Some(("11.1.1".into(), "c".into(), Some("ab".into()))));
    }
}

#[cfg(test)]
mod live {
    /// Real download into OBELISK_PRISM_TEST_DIR (run by hand: cargo test --lib prismget::live -- --ignored).
    #[test]
    #[ignore]
    fn downloads_and_unpacks() {
        let dir = std::path::PathBuf::from(std::env::var("OBELISK_PRISM_TEST_DIR").expect("set OBELISK_PRISM_TEST_DIR"));
        let dest = dir.join("PrismLauncher");
        let mut calls = 0;
        let ver = super::install(&dest, |_, _| calls += 1).expect("install");
        assert!(dest.join("prismlauncher.exe").is_file(), "no exe");
        assert!(!dest.join("portable.txt").exists(), "portable marker left");
        assert!(calls > 0);
        assert!(super::install(&dest, |_, _| ()).is_err(), "second install must refuse");
        println!("Prism {ver} at {}", dest.display());
    }
}
