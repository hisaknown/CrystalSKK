//! COM のクラス登録。
//!
//! 登録先は `HKEY_LOCAL_MACHINE\Software\Classes` である。入力方式の登録
//! そのものが機械全体に書かれるため、COM のクラスだけを利用者ごとにしても
//! 意味がない (ADR-0007)。したがって登録には管理者権限が要る。
//!
//! 解除は利用者ごとの領域も見る。ADR-0006 の時期に書かれたものが
//! 残っている可能性があるため。

use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::{Error, GUID, HSTRING, PCWSTR, Result};

use crate::guids::{CLASS_DESCRIPTION, CLSID_CRYSTALSKK};

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` 形式の文字列。
///
/// レジストリのキー名はこの形でなければならない。
pub fn guid_to_string(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

/// この DLL の場所。
pub fn module_path(module: HMODULE) -> Result<String> {
    let mut buffer = [0u16; 512];
    // SAFETY: 長さ付きのバッファをそのまま渡している。
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) };
    if length == 0 {
        // SAFETY: 直前の呼び出しの失敗理由を読むだけ。
        let code = unsafe { windows::Win32::Foundation::GetLastError() };
        return Err(Error::from_hresult(code.to_hresult()));
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

/// COM のクラスとして、指定した場所の DLL を登録する。
///
/// 場所を引数で受け取るのは、セットアップツールが**自分ではない DLL** を
/// 登録できるようにするため。DLL が自分を登録するときは
/// [`module_path()`] で自分の場所を調べて渡す。
pub fn register_class(dll_path: &str) -> Result<()> {
    let key = class_key();
    write_string(HKEY_LOCAL_MACHINE, &key, None, CLASS_DESCRIPTION)?;

    let server = format!(r"{key}\InprocServer32");
    write_string(HKEY_LOCAL_MACHINE, &server, None, dll_path)?;
    // TSF の TIP は常にアパートメントスレッドで動く。
    write_string(
        HKEY_LOCAL_MACHINE,
        &server,
        Some("ThreadingModel"),
        "Apartment",
    )?;
    Ok(())
}

/// COM のクラスを置くキー。
fn class_key() -> String {
    let clsid = guid_to_string(&CLSID_CRYSTALSKK);
    format!(r"Software\Classes\CLSID\{clsid}")
}

/// クラス登録を消す。登録されていなくても成功とみなす。
///
/// 利用者ごとの領域も見るのは、過去の版がそちらへ書いていたため。
pub fn unregister_class() -> Result<()> {
    let key = HSTRING::from(class_key());
    let machine = delete_tree(HKEY_LOCAL_MACHINE, &key);
    let per_user = delete_tree(HKEY_CURRENT_USER, &key);
    // どちらかが消せていれば十分。両方失敗したときだけ報告する。
    machine.or(per_user)
}

/// 利用者ごとのクラス登録だけを消す。
///
/// COM は `HKEY_CURRENT_USER` を `HKEY_LOCAL_MACHINE` より先に見る。
/// 利用者ごとの登録が残っていると、機械全体へ入れ直しても古い DLL が
/// 使われ続ける。導入のたびにこれを消しておく必要がある。
pub fn unregister_per_user_class() -> Result<()> {
    let key = HSTRING::from(class_key());
    delete_tree(HKEY_CURRENT_USER, &key)
}

/// 利用者ごとに登録されている DLL の場所。
pub fn per_user_dll_path() -> Option<String> {
    read_dll_path(HKEY_CURRENT_USER)
}

/// 機械全体に登録されている DLL の場所。
pub fn machine_dll_path() -> Option<String> {
    read_dll_path(HKEY_LOCAL_MACHINE)
}

fn delete_tree(root: HKEY, key: &HSTRING) -> Result<()> {
    // SAFETY: 根のハンドルは定数で、キー名は有効な文字列。
    let status = unsafe { RegDeleteTreeW(root, key) };
    // 消えていればそれでよい (2 = ERROR_FILE_NOT_FOUND)。
    if status == ERROR_SUCCESS || status.0 == 2 {
        Ok(())
    } else {
        Err(Error::from_hresult(status.to_hresult()))
    }
}

/// キーを作り、文字列の値を書く。`name` が `None` なら既定の値。
fn write_string(root: HKEY, key: &str, name: Option<&str>, value: &str) -> Result<()> {
    let key = HSTRING::from(key);
    let mut handle = HKEY::default();

    // SAFETY: 出力先のハンドルは有効な場所を指しており、成功したときだけ使う。
    let status = unsafe {
        RegCreateKeyExW(
            root,
            &key,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(Error::from_hresult(status.to_hresult()));
    }

    let name = name.map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));

    // REG_SZ は終端の NUL を含めた長さで書く。
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(&data[..]))
    };

    // SAFETY: バイト列は `data` の中身そのもので、この呼び出しの間生きている。
    let status = unsafe { RegSetValueExW(handle, name, None, REG_SZ, Some(bytes)) };
    // SAFETY: 直前に開いたハンドルをここで閉じる。
    unsafe {
        let _ = RegCloseKey(handle);
    }

    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(Error::from_hresult(status.to_hresult()))
    }
}

/// 登録されている DLL の場所。登録されていなければ `None`。
///
/// 機械全体の登録を先に見て、次に利用者ごとの登録を見る。
pub fn registered_dll_path() -> Option<String> {
    read_dll_path(HKEY_LOCAL_MACHINE).or_else(|| read_dll_path(HKEY_CURRENT_USER))
}

fn read_dll_path(root: HKEY) -> Option<String> {
    let key = HSTRING::from(format!(r"{}\InprocServer32", class_key()));
    let mut handle = HKEY::default();

    // SAFETY: 出力先のハンドルは有効な場所を指す。
    let status = unsafe { RegOpenKeyExW(root, &key, None, KEY_READ, &mut handle) };
    if status != ERROR_SUCCESS {
        return None;
    }

    let mut buffer = [0u16; 512];
    let mut size = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;
    // SAFETY: 長さを渡して書き込ませ、書かれた長さだけを読む。
    let status = unsafe {
        RegQueryValueExW(
            handle,
            PCWSTR::null(),
            None,
            None,
            Some(buffer.as_mut_ptr().cast::<u8>()),
            Some(&mut size),
        )
    };
    // SAFETY: 直前に開いたハンドルを閉じる。
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if status != ERROR_SUCCESS {
        return None;
    }

    let chars = (size as usize) / std::mem::size_of::<u16>();
    let text = String::from_utf16_lossy(&buffer[..chars]);
    let text = text.trim_end_matches('\0').to_owned();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_guids_the_way_the_registry_wants() {
        assert_eq!(
            guid_to_string(&CLSID_CRYSTALSKK),
            "{5CD1C143-735E-4051-986E-69864A08FEBE}"
        );
    }
}
