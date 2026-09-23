//! 名前付きパイプの上を、一行ずつ運ぶ。
//!
//! 運び方はここだけに閉じてある。**何を運ぶか**は [`crystalskk_ipc`] が、
//! **何と答えるか**は [`crate::service`] が決める。
//!
//! # 一件ずつ順に答える
//!
//! 受けるのは一つずつで、答えてから次を待つ。引くのに 1 マイクロ秒も
//! かからないので、並べて捌く必要がない。**捌かないぶん、辞書を守る錠も
//! 要らない。**

use std::io;

use windows::Win32::Foundation::{ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::Pipes::ConnectNamedPipe;
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, SetNamedPipeHandleState, WaitNamedPipeW,
};
use windows::core::HSTRING;

use crate::security::PipeSecurity;

/// 一度に運ぶ上限。見出し語も候補もこれを超えない。
const BUFFER: usize = 64 * 1024;

/// 待ち合わせ場所を開き、頼みを受け続ける。
#[derive(Debug)]
pub struct Listener {
    handle: OwnedHandle,
    /// 許可はパイプより長生きさせる。
    _security: PipeSecurity,
}

impl Listener {
    /// パイプを開く。
    pub fn open(name: &str) -> io::Result<Self> {
        let security = PipeSecurity::new()?;
        let wide = HSTRING::from(name);

        // SAFETY: 名前も許可もこの関数で用意したもの。
        let handle = unsafe {
            CreateNamedPipeW(
                &wide,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                u32::try_from(BUFFER).unwrap_or(0),
                u32::try_from(BUFFER).unwrap_or(0),
                0,
                Some(security.attributes()),
            )
        };
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle: OwnedHandle(handle),
            _security: security,
        })
    }

    /// 頼みが一つ来るまで待ち、`answer` に答えさせる。
    ///
    /// 戻り値は `answer` が返したもの。相手が途中で消えても、**待ち続ける
    /// 側は死なない**。
    pub fn serve_one<T>(&self, answer: impl FnOnce(&str) -> (String, T)) -> io::Result<Option<T>> {
        // SAFETY: 持ち手は自分で開いたパイプ。
        unsafe {
            // すでに繋がっている相手がいる場合もある。それは失敗ではない。
            if let Err(e) = ConnectNamedPipe(self.handle.0, None)
                && e.code() != windows::core::HRESULT::from(ERROR_PIPE_CONNECTED)
            {
                return Err(io::Error::other(e.message()));
            }

            let outcome = (|| -> io::Result<T> {
                let line = read_message(self.handle.0)?;
                let (reply, value) = answer(&line);
                write_message(self.handle.0, &reply)?;
                // **相手が読み終わるまで待つ。** 切るのが先になると、
                // まだ渡っていない答えが捨てられる。
                let _ = FlushFileBuffers(self.handle.0);
                Ok(value)
            })();

            let _ = DisconnectNamedPipe(self.handle.0);
            match outcome {
                Ok(value) => Ok(Some(value)),
                // 途中で切れた相手のために止まる理由はない。
                Err(_) => Ok(None),
            }
        }
    }
}

/// 頼みを一つ送り、答えを受け取る。
///
/// 一回ごとに繋いで切る。**握ったままにしないので、サーバが入れ替わっても
/// 次から新しいほうに繋がる。**
pub fn ask(name: &str, request: &str, wait_ms: u32) -> io::Result<String> {
    let wide = HSTRING::from(name);

    // SAFETY: 名前はこの関数で用意したもの。開いた持ち手は必ず閉じる。
    unsafe {
        let handle = OwnedHandle(connect(&wide, wait_ms)?);

        let mode = PIPE_READMODE_MESSAGE | PIPE_WAIT;
        SetNamedPipeHandleState(handle.0, Some(&mode), None, None)
            .map_err(|e: windows::core::Error| io::Error::other(e.message()))?;

        write_message(handle.0, request)?;
        read_message(handle.0)
    }
}

/// 繋がるまで試す。
///
/// **一度きりでは足りない。** サーバは一件ずつ順に答えるので、別のアプリが
/// 話している間は「使用中」で断られる。答えるのは一瞬なので、待てばすぐ
/// 空く。
///
/// `WaitNamedPipeW` は「空きができた」ことしか言わない。**その空きを別の
/// 誰かが先に取ることがある**ので、取れるまで繰り返す。MSDN が示す作法も
/// これである。
///
/// # Safety
///
/// `name` が有効な文字列であること。
unsafe fn connect(name: &HSTRING, wait_ms: u32) -> io::Result<HANDLE> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(u64::from(wait_ms));
    loop {
        // SAFETY: 呼び出し側の約束による。
        let opened = unsafe {
            CreateFileW(
                name,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                // **サーバにこちらの身分を借りさせない。** TIP は他人の
                // プロセスの中で動くので、なりすましの踏み台にされては困る。
                FILE_FLAGS_AND_ATTRIBUTES(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0),
                None,
            )
        };
        match opened {
            Ok(handle) => return Ok(handle),
            Err(e) if e.code() != windows::core::HRESULT::from(ERROR_PIPE_BUSY) => {
                return Err(io::Error::other(e.message()));
            }
            Err(_) => {}
        }

        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(io::Error::other("辞書サーバが取り込み中です"));
        }
        // SAFETY: 呼び出し側の約束による。空くまで待つ。
        unsafe {
            let _ = WaitNamedPipeW(name, u32::try_from(left.as_millis()).unwrap_or(u32::MAX));
        }
    }
}

/// 一つの塊を読む。
///
/// # Safety
///
/// `handle` がメッセージ型のパイプであること。
unsafe fn read_message(handle: HANDLE) -> io::Result<String> {
    let mut buffer = vec![0u8; BUFFER];
    let mut read = 0u32;
    // SAFETY: 呼び出し側の約束による。読む先は手元の入れ物。
    unsafe {
        ReadFile(handle, Some(&mut buffer), Some(&mut read), None)
            .map_err(|e: windows::core::Error| io::Error::other(e.message()))?;
    }
    buffer.truncate(read as usize);
    String::from_utf8(buffer).map_err(io::Error::other)
}

/// 一つの塊を書く。
///
/// # Safety
///
/// `handle` がメッセージ型のパイプであること。
unsafe fn write_message(handle: HANDLE, text: &str) -> io::Result<()> {
    let mut written = 0u32;
    // SAFETY: 呼び出し側の約束による。書く元は手元の文字列。
    unsafe {
        WriteFile(handle, Some(text.as_bytes()), Some(&mut written), None)
            .map_err(|e: windows::core::Error| io::Error::other(e.message()))?;
    }
    Ok(())
}

/// 閉じ忘れないための持ち手。
#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: 自分で開いたものをここで閉じる。
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
