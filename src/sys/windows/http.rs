//! Minimal blocking HTTPS GET on top of WinHTTP.

use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};

pub fn http_get(host: &str, path: &str) -> Result<String, String> {
    unsafe { http_get_impl(host, path) }
}

unsafe fn http_get_impl(host: &str, path: &str) -> Result<String, String> {
    let host_w = encode_wide(host);
    let path_w = encode_wide(path);
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
        h_connect = WinHttpConnect(h_session, PCWSTR(host_w.as_ptr()), 443, 0);
        if h_connect.is_null() {
            return Err("WinHttpConnect failed".to_string());
        }

        h_request = WinHttpOpenRequest(
            h_connect,
            windows::core::w!("GET"),
            PCWSTR(path_w.as_ptr()),
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

        Ok(String::from_utf8_lossy(&body_bytes).to_string())
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
