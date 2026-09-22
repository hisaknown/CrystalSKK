//! 導入と削除の手順。
//!
//! 行うことは三つしかない。DLL を置き場所へ写し、COM のクラスとして
//! 登録し、入力方式として登録する。削除はその逆をたどる。
//!
//! 置き場所は `%ProgramFiles%` である。登録が機械全体に書かれる以上
//! (ADR-0007)、DLL も機械全体から見える場所になければ辻褄が合わない。
//! 書き込みに管理者権限が要ることは、ここでは利点でもある。全利用者の
//! あらゆるプロセスに読み込まれる DLL を、権限のない者が差し替えられては
//! ならない。
//!
//! ビルド成果物を直接登録しないのは、使用中の DLL がビルドに掴まれて
//! 作り直せなくなるのを避けるため。

use std::io;
use std::path::{Path, PathBuf};

use crystalskk_tip::{profile, registry};

/// 導入した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// 実際に登録した DLL の場所。
    pub dll: PathBuf,
    /// 入れ替えのために古い DLL を退けたか。
    pub replaced: bool,
}

/// 今の導入状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// 登録されていない。
    NotInstalled,
    /// 登録されている。
    Installed {
        dll: PathBuf,
        /// 登録されている場所に DLL が実在するか。
        present: bool,
    },
}

/// 置き場所。`%ProgramFiles%\CrystalSKK\bin`。
pub fn install_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("ProgramFiles")
        .ok_or_else(|| io::Error::other("ProgramFiles が設定されていません"))?;
    Ok(PathBuf::from(base).join("CrystalSKK").join("bin"))
}

/// 登録に使う DLL の名前。
pub const DLL_NAME: &str = "crystalskk_tip.dll";

/// 導入する。
///
/// `source` の DLL を置き場所へ写してから登録する。すでに同じ場所へ
/// 登録されている場合は、上書きして登録し直す。
pub fn install(source: &Path) -> io::Result<Installed> {
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("DLL が見つかりません: {}", source.display()),
        ));
    }

    let directory = install_dir()?;
    std::fs::create_dir_all(&directory)?;
    let destination = directory.join(DLL_NAME);
    let replaced = destination.exists();

    // 同じ場所を指しているなら写す必要はない。
    let same = std::fs::canonicalize(source)
        .ok()
        .zip(std::fs::canonicalize(&destination).ok())
        .is_some_and(|(a, b)| a == b);
    if !same {
        std::fs::copy(source, &destination).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "{} へ写せません: {e}。\
                     すでに導入済みなら、この DLL を読み込んでいるアプリを閉じてから試してください。",
                    destination.display()
                ),
            )
        })?;
    }

    let path = destination.to_string_lossy().into_owned();
    registry::register_class(&path).map_err(|e| to_io("COM のクラス登録", e))?;
    profile::register_profile(&path).map_err(|e| to_io("入力方式の登録", e))?;

    Ok(Installed {
        dll: destination,
        replaced,
    })
}

/// 削除する。`purge` が真なら写した DLL も消す。
///
/// 登録されていなくても失敗にしない。中途半端な状態からでも、呼べば
/// きれいになることを優先する。
pub fn uninstall(purge: bool) -> io::Result<()> {
    let profile = profile::unregister_profile();
    let class = registry::unregister_class();

    if purge && let Ok(directory) = install_dir() {
        let dll = directory.join(DLL_NAME);
        if dll.exists() {
            std::fs::remove_file(&dll).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "{} を消せません: {e}。\
                         読み込んでいるアプリを閉じるか、サインインし直してから試してください。",
                        dll.display()
                    ),
                )
            })?;
        }
        // 空になったときだけ片付ける。他のものが入っていれば触らない。
        let _ = std::fs::remove_dir(&directory);
    }

    profile.and(class).map_err(|e| to_io("登録の解除", e))
}

/// 導入されているか調べる。
pub fn status() -> Status {
    match registry::registered_dll_path() {
        Some(path) => {
            let dll = PathBuf::from(path);
            let present = dll.is_file();
            Status::Installed { dll, present }
        }
        None => Status::NotInstalled,
    }
}

/// Windows の失敗を、何をしていたかが分かる形に直す。
///
/// HRESULT をそのまま出すのは不親切だが、消してしまうともっと困る。
/// 権限が足りない場合は、それと分かる言葉を添える。
fn to_io(step: &str, error: windows::core::Error) -> io::Error {
    let code = error.code();
    let mut message = format!("{step}に失敗しました ({code:?}): {}", error.message());
    if is_access_denied(code) {
        message.push_str(
            "
管理者権限が要ります。管理者として実行した PowerShell から試してください。",
        );
    }
    io::Error::other(message)
}

/// 権限不足を表す HRESULT か。
fn is_access_denied(code: windows::core::HRESULT) -> bool {
    // E_ACCESSDENIED と、Win32 の ERROR_ACCESS_DENIED を包んだもの。
    code == windows::Win32::Foundation::E_ACCESSDENIED
        || code == windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_directory_sits_under_program_files() {
        let directory = install_dir().expect("ProgramFiles がある");
        assert!(directory.ends_with(Path::new("CrystalSKK").join("bin")));
    }

    #[test]
    fn installing_a_missing_file_fails_before_touching_the_registry() {
        let error = install(Path::new("存在しない.dll")).expect_err("失敗する");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
