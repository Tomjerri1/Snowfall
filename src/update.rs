pub fn parse_version(s: &str) -> Option<(u64, u64, u64, bool)> {
    let s = s.trim();
    let s = if let Some(stripped) = s.strip_prefix('v').or_else(|| s.strip_prefix('V')) {
        stripped
    } else {
        s
    };
    let s = s.split('+').next().unwrap_or(s);

    let (version_part, is_prerelease) = if let Some(idx) = s.find('-') {
        let v = &s[..idx];
        let suf = &s[idx + 1..];
        if suf.is_empty() {
            return None;
        }
        (v, true)
    } else {
        (s, false)
    };

    if version_part.contains(' ') {
        return None;
    }

    let parts: Vec<&str> = version_part.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    for part in &parts {
        if part.is_empty() {
            return None;
        }
        if part.starts_with('0') && part.len() > 1 {
            return None;
        }
        if !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }

    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;

    Some((major, minor, patch, is_prerelease))
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    let (curr_maj, curr_min, curr_pat, curr_pre) = match parse_version(current) {
        Some(v) => v,
        None => return false,
    };
    let (latest_maj, latest_min, latest_pat, latest_pre) = match parse_version(latest) {
        Some(v) => v,
        None => return false,
    };

    if latest_maj != curr_maj {
        return latest_maj > curr_maj;
    }
    if latest_min != curr_min {
        return latest_min > curr_min;
    }
    if latest_pat != curr_pat {
        return latest_pat > curr_pat;
    }

    curr_pre && !latest_pre
}

pub fn extract_tag_name(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let mut search_idx = 0;
    while let Some(pos) = json[search_idx..].find(key) {
        let mut idx = search_idx + pos + key.len();
        while idx < json.len() && json.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < json.len() && json.as_bytes()[idx] == b':' {
            idx += 1;
            while idx < json.len() && json.as_bytes()[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx < json.len() && json.as_bytes()[idx] == b'"' {
                idx += 1;
                let mut escaped = false;
                let mut val = String::new();
                while idx < json.len() {
                    let b = json.as_bytes()[idx];
                    if escaped {
                        val.push(b as char);
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        return Some(val);
                    } else {
                        val.push(b as char);
                    }
                    idx += 1;
                }
            }
        }
        search_idx += pos + key.len();
    }
    None
}

const LATEST_RELEASE_HOST: &str = "api.github.com";
const LATEST_RELEASE_PATH: &str = "/repos/Tomjerri1/Snowfall/releases/latest";

/// Fetches the tag of the latest GitHub release through the platform HTTP client.
pub fn fetch_latest_tag() -> Result<String, String> {
    let body = crate::sys::http_get(LATEST_RELEASE_HOST, LATEST_RELEASE_PATH)?;
    extract_tag_name(&body).ok_or_else(|| "tag not found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        let v1 = parse_version("v0.1.2");
        assert!(v1 == Some((0, 1, 2, false)));

        let v2 = parse_version("0.1.2-beta.1");
        assert!(v2 == Some((0, 1, 2, true)));

        let v3 = parse_version("abc");
        assert!(v3.is_none());
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.1.1", "v0.1.2"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(is_newer("0.1.2-beta.1", "0.1.2"));
        assert!(!is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("0.1.9", "0.1.10"));
    }

    #[test]
    fn test_extract_tag_name() {
        let json1 = r#"{"url":"x","tag_name": "v0.1.2","name":"n"}"#;
        assert!(extract_tag_name(json1) == Some("v0.1.2".to_string()));

        let json2 = "{}";
        assert!(extract_tag_name(json2).is_none());
    }

    #[test]
    fn test_build_metadata_and_bad_tags() {
        assert!(parse_version("v0.1.2+build.1") == Some((0, 1, 2, false)));
        assert!(parse_version("0.1.2-pr.1+build-5") == Some((0, 1, 2, true)));
        assert!(parse_version("release-0.1.2").is_none());
        assert!(is_newer("0.1.1", "v0.1.2+build.1"));
    }
}
