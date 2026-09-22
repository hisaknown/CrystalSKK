//! WinHTTP による HTTP 取得。
//!
//! OS の HTTP スタックを使う理由は ADR-0004 を参照。要約すると、TLS の
//! 実装を自前で抱えないためである。証明書の検証もプロキシ設定も Windows
//! のものがそのまま使われる。
//!
//! このモジュールだけが `unsafe` を含む。

use std::ffi::c_void;

use windows::Win32::Networking::WinHttp::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::url::Url;
use crate::{Downloaded, Error, Fetched};

/// 一度に読み出す大きさ。
const READ_CHUNK: usize = 64 * 1024;

/// 取得を諦めるまでの時間 (ミリ秒)。
const TIMEOUT_MS: i32 = 30_000;

/// `HINTERNET` の後始末を保証する持ち手。
struct Handle(*mut c_void);

impl Handle {
    /// 生ハンドルを受け取る。null なら直前の失敗として扱う。
    fn new(raw: *mut c_void) -> Result<Self, Error> {
        if raw.is_null() {
            Err(Error::from_last_os_error())
        } else {
            Ok(Self(raw))
        }
    }

    fn as_raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // 後始末に失敗しても打つ手がない。取得結果には影響しない。
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

pub fn get(url: &str, etag: Option<&str>) -> Result<Fetched, Error> {
    let url = Url::parse(url)?;

    // SAFETY: 以下の呼び出しはいずれも WinHTTP の定める手順どおりに並んでおり、
    // 渡すハンドルは `Handle` が生存を保証している。文字列は呼び出しの間だけ
    // 参照され、いずれも呼び出しより長く生きる変数に束ねてある。
    unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("CrystalSKK"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(
            session.as_raw(),
            TIMEOUT_MS,
            TIMEOUT_MS,
            TIMEOUT_MS,
            TIMEOUT_MS,
        )?;

        let host = HSTRING::from(url.host.as_str());
        let connect = Handle::new(WinHttpConnect(session.as_raw(), &host, url.port, 0))?;

        let target = HSTRING::from(url.target.as_str());
        let flags = if url.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = Handle::new(WinHttpOpenRequest(
            connect.as_raw(),
            w!("GET"),
            &target,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ))?;

        if let Some(etag) = etag {
            let header: Vec<u16> = format!("If-None-Match: {etag}").encode_utf16().collect();
            WinHttpAddRequestHeaders(request.as_raw(), &header, WINHTTP_ADDREQ_FLAG_ADD)?;
        }

        WinHttpSendRequest(request.as_raw(), None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.as_raw(), std::ptr::null_mut())?;

        let status = query_status(&request)?;
        if status == 304 {
            return Ok(Fetched::NotModified);
        }
        if status != 200 {
            return Err(Error::Http(status));
        }

        let etag = query_header(&request, WINHTTP_QUERY_ETAG);
        let body = read_body(&request)?;
        Ok(Fetched::Downloaded(Downloaded { body, etag }))
    }
}

/// 応答のステータスコード。
unsafe fn query_status(request: &Handle) -> Result<u16, Error> {
    let mut status: u32 = 0;
    let mut length = u32::try_from(size_of::<u32>()).expect("4 は u32 に収まる");
    let mut index = 0u32;
    unsafe {
        WinHttpQueryHeaders(
            request.as_raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(std::ptr::from_mut(&mut status).cast::<c_void>()),
            &mut length,
            &mut index,
        )?;
    }
    u16::try_from(status).map_err(|_| Error::Http(0))
}

/// 応答ヘッダを一つ読む。無ければ `None`。
unsafe fn query_header(request: &Handle, info_level: u32) -> Option<String> {
    let mut length = 0u32;
    let mut index = 0u32;

    // 一度目は長さを知るためだけに呼ぶ。バッファ不足で失敗するのが正常。
    let probe = unsafe {
        WinHttpQueryHeaders(
            request.as_raw(),
            info_level,
            PCWSTR::null(),
            None,
            &mut length,
            &mut index,
        )
    };
    if probe.is_ok() || length == 0 {
        return None;
    }

    let mut buffer = vec![0u16; (length as usize).div_ceil(size_of::<u16>())];
    let mut index = 0u32;
    unsafe {
        WinHttpQueryHeaders(
            request.as_raw(),
            info_level,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            &mut length,
            &mut index,
        )
        .ok()?;
    }

    let text = String::from_utf16_lossy(&buffer);
    let text = text.trim_end_matches('\0').to_owned();
    if text.is_empty() { None } else { Some(text) }
}

/// 本文を最後まで読む。
unsafe fn read_body(request: &Handle) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                request.as_raw(),
                chunk.as_mut_ptr().cast::<c_void>(),
                u32::try_from(chunk.len()).expect("読み出し単位は u32 に収まる"),
                &mut read,
            )?;
        }
        if read == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&chunk[..read as usize]);
    }
}
