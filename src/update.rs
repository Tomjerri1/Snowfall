use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};

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

pub fn fetch_latest_tag() -> Result<String, String> {
    unsafe { fetch_latest_tag_impl() }
}

unsafe fn fetch_latest_tag_impl() -> Result<String, String> {
    let h_session = WinHttpOpen(
        windows::core::w!("Snowfall-update-check"),
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        PCWSTR::null(),
        PCWSTR::null(),
        0,
    );
    if h_session.is_null() {
        return Err("WinHttpOpen failed".to_string());
    }

    let mut h_connect = std::ptr::null_mut();
    let mut h_request = std::ptr::null_mut();

    let body_res = (|| -> Result<String, String> {
        h_connect = WinHttpConnect(h_session, windows::core::w!("api.github.com"), 443, 0);
        if h_connect.is_null() {
            return Err("WinHttpConnect failed".to_string());
        }

        h_request = WinHttpOpenRequest(
            h_connect,
            windows::core::w!("GET"),
            windows::core::w!("/repos/Tomjerri1/Snowfall/releases/latest"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if h_request.is_null() {
            return Err("WinHttpOpenRequest failed".to_string());
        }

        let headers =
            encode_wide("Accept: application/vnd.github+json\r\nUser-Agent: Snowfall-update-check");
        WinHttpAddRequestHeaders(
            h_request,
            &headers,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        )
        .map_err(|e| format!("WinHttpAddRequestHeaders failed: {}", e))?;

        let _ = WinHttpSetTimeouts(h_session, 5000, 5000, 5000, 5000);

        WinHttpSendRequest(h_request, None, None, 0, 0, 0)
            .map_err(|e| format!("WinHttpSendRequest failed: {}", e))?;

        WinHttpReceiveResponse(h_request, std::ptr::null_mut())
            .map_err(|e| format!("WinHttpReceiveResponse failed: {}", e))?;

        let mut status_code: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        let mut index = 0;
        WinHttpQueryHeaders(
            h_request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            windows::core::PCWSTR::null(),
            Some(&mut status_code as *mut u32 as *mut std::ffi::c_void),
            &mut status_size,
            &mut index,
        )
        .map_err(|e| format!("WinHttpQueryHeaders failed: {}", e))?;

        if status_code != 200 {
            return Err(format!("HTTP status: {}", status_code));
        }

        let mut body_bytes = Vec::new();
        let mut buffer = [0u8; 65536];
        loop {
            let mut bytes_read = 0u32;
            WinHttpReadData(
                h_request,
                buffer.as_mut_ptr() as *mut std::ffi::c_void,
                buffer.len() as u32,
                &mut bytes_read,
            )
            .map_err(|e| format!("WinHttpReadData failed: {}", e))?;

            if bytes_read == 0 {
                break;
            }
            body_bytes.extend_from_slice(&buffer[..bytes_read as usize]);
        }

        let body = String::from_utf8_lossy(&body_bytes).to_string();
        extract_tag_name(&body).ok_or_else(|| "tag not found".to_string())
    })();

    if !h_request.is_null() {
        let _ = WinHttpCloseHandle(h_request);
    }
    if !h_connect.is_null() {
        let _ = WinHttpCloseHandle(h_connect);
    }
    if !h_session.is_null() {
        let _ = WinHttpCloseHandle(h_session);
    }

    body_res
}

fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
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
