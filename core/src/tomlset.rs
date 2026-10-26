//! Set one key in a TOML file's text, keeping its indent, comments and everything else byte-for-byte.

/// `key` is a dotted path ("section.key"); `value` is TOML as written (true, 12, "text").
/// Fails when the key isn't there, so a typo can't turn an A/B into a silent no-op.
pub fn set_key(text: &str, key: &str, value: &str) -> Result<String, String> {
    let new_value: toml::Value = format!("v = {value}").parse::<toml::Table>()
        .map_err(|_| format!("{key}: {value} isn't a TOML value (quote strings)"))?
        .remove("v").ok_or("no value")?;
    let mut section = String::new();
    let mut out = String::with_capacity(text.len() + value.len());
    let mut found = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            section = t.trim_start_matches('[').split(']').next().unwrap_or_default().trim().to_string();
            section = unquote_path(&section);
        } else if let Some(eq) = t.find('=').filter(|_| !t.starts_with('#')) {
            let name = unquote_path(t[..eq].trim());
            let full = if section.is_empty() { name } else { format!("{section}.{name}") };
            if full == key {
                if found { return Err(format!("{key} is set twice")); }
                let start = line.find('=').unwrap() + 1;
                let rest = &line[start..];
                let lead = rest.len() - rest.trim_start().len();
                let end = start + lead + value_len(rest.trim_start()).ok_or(format!("{key}: multi-line values aren't supported"))?;
                out.push_str(&line[..start + lead]);
                out.push_str(value);
                out.push_str(&line[end..]);
                found = true;
                continue;
            }
        }
        out.push_str(line);
    }
    if !found { return Err(format!("{key} isn't in the file")); }
    // Check by reading it back: the key must now parse to the new value.
    let table: toml::Table = out.parse().map_err(|e| format!("{key}: the edit broke the file: {e}"))?;
    let mut v = &toml::Value::Table(table);
    for part in key.split('.') { v = v.get(part).ok_or(format!("{key}: not found after the edit"))?; }
    if *v != new_value { return Err(format!("{key}: reads back as {v}, not {value}")); }
    Ok(out)
}

fn unquote_path(p: &str) -> String {
    p.split('.').map(|s| s.trim().trim_matches(['"', '\''])).collect::<Vec<_>>().join(".")
}

/// Length of the value at the start of `s` (strings, arrays and inline tables on one line); None if it runs on.
fn value_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i, mut quote) = (0i32, 0, None::<u8>);
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == b'\\' && q == b'"' { i += 1; } else if c == q { quote = None; if depth == 0 { return Some(i + 1); } }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'[' | b'{' => depth += 1,
                b']' | b'}' => { depth -= 1; if depth == 0 { return Some(i + 1); } }
                b'#' | b'\r' | b'\n' if depth == 0 => return Some(s[..i].trim_end().len()),
                _ => {}
            },
        }
        i += 1;
    }
    (depth == 0 && quote.is_none()).then(|| s.trim_end().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "# PatchBox\r\n[modules]\r\n\t# Villager API\r\n\tvillagerapi = false # off by default\r\n\tname = \"a # b\"\r\n\tlist = [1, 2]\r\n\r\n[\"other\".deep]\r\n\tx = 3\r\n";

    #[test]
    fn sets_and_keeps_the_rest() {
        let s = set_key(T, "modules.villagerapi", "true").unwrap();
        assert_eq!(s, T.replace("villagerapi = false", "villagerapi = true"));
        let s = set_key(T, "modules.name", "\"c\"").unwrap();
        assert!(s.contains("\tname = \"c\"\r\n"));
        let s = set_key(T, "modules.list", "[3]").unwrap();
        assert!(s.contains("\tlist = [3]\r\n"));
        let s = set_key(T, "other.deep.x", "4").unwrap();
        assert!(s.ends_with("\tx = 4\r\n"));
    }

    #[test]
    fn refuses_missing_keys_and_bad_values() {
        assert!(set_key(T, "modules.villagerAPI", "true").is_err());
        assert!(set_key(T, "villagerapi", "true").is_err());
        assert!(set_key(T, "modules.name", "c").is_err());
        assert!(set_key("[a]\nx = [\n1,\n]\n", "a.x", "[2]").is_err());
    }
}

#[cfg(test)]
mod real {
    /// OBELISK_TEST_TOML=<file> OBELISK_TEST_KEY=<section.key> cargo test --lib real_file -- --ignored
    #[test]
    #[ignore]
    fn real_file() {
        let text = std::fs::read_to_string(std::env::var("OBELISK_TEST_TOML").unwrap()).unwrap();
        let key = std::env::var("OBELISK_TEST_KEY").unwrap();
        let off = super::set_key(&text, &key, "false").unwrap();
        assert_eq!(text.lines().zip(off.lines()).filter(|(a, b)| a != b).count(), 1, "exactly one line changes");
        assert_eq!(super::set_key(&off, &key, "true").unwrap(), text, "setting it back restores every byte");
    }
}
