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
//!
//! # 使用中の DLL を入れ替える
//!
//! 読み込まれている DLL は上書きも削除もできない。だが**改名はできる**。
//! 動いているプロセスはファイルの名前ではなく実体を掴んでいるため、
//! 名前が変わっても困らない。
//!
//! そこで、古いものを退けてから新しいものを同じ名前で置く。登録した
//! 場所は変わらず、サインインし直す必要もない。退けた残骸は次の導入の
//! ついでに消す。そのときも使用中なら消せないが、いずれ消える。

use std::io;
use std::path::{Path, PathBuf};

use crystalskk_tip::{icon, profile, registry};
use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows::core::HSTRING;

/// 導入した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// 写した元の DLL。
    pub source: PathBuf,
    /// 実際に登録した DLL の場所。
    pub dll: PathBuf,
    /// 入れ替えのために古い DLL を退けたか。
    pub replaced: bool,
    /// 使用中だったので、古い DLL を別名へ退けたか。
    ///
    /// 退けた場合、すでに動いているアプリは古いほうを使い続ける。
    pub retired: bool,
    /// 利用者ごとの古い登録を消したか。
    pub cleared_per_user: bool,
}

/// 今の導入状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// 機械全体に登録されている DLL。
    pub machine: Option<PathBuf>,
    /// 利用者ごとに登録されている DLL。これがあると機械全体の登録より優先される。
    pub per_user: Option<PathBuf>,
    /// 32 ビットのアプリ向けに登録されている DLL。
    pub wow32: Option<PathBuf>,
}

impl Status {
    /// 実際に使われる DLL。COM は利用者ごとの登録を先に見る。
    pub fn effective(&self) -> Option<&PathBuf> {
        self.per_user.as_ref().or(self.machine.as_ref())
    }

    pub fn is_installed(&self) -> bool {
        self.effective().is_some()
    }
}

/// 置き場所。`%ProgramFiles%\CrystalSKK\bin`。
pub fn install_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("ProgramFiles")
        .ok_or_else(|| io::Error::other("ProgramFiles が設定されていません"))?;
    Ok(PathBuf::from(base).join("CrystalSKK").join("bin"))
}

/// 32 ビットの DLL の置き場所。`bin` の下の `x86`。
///
/// 名前は 64 ビットのものと同じにする。退ける・片付ける仕組みを
/// そのまま使える。
pub fn wow32_dir(directory: &Path) -> PathBuf {
    directory.join("x86")
}

/// 登録に使う DLL の名前。
pub const DLL_NAME: &str = "crystalskk_tip.dll";

/// 設定画面に出す絵の名前。
pub const ICON_NAME: &str = "crystalskk.ico";

/// 導入する。
///
/// `source` の DLL を置き場所へ写してから登録する。すでに同じ場所へ
/// 登録されている場合は、上書きして登録し直す。
/// 辞書サーバを入れ替える。
///
/// **先に終わってもらう。** 動いている exe は上書きできないうえ、落として
/// しまうと書きかけの学習が消える。
///
/// 置けなくても導入そのものは続ける。サーバが古いままでも、語彙が同じなら
/// 話は通じる。
pub fn install_server(source: &Path, directory: &Path) -> io::Result<bool> {
    let destination = directory.join(crate::server::SERVER_NAME);
    let stopped = crate::server::stop();
    // 止めたので、前に退けたサーバはもう誰も動かしていないはずである。
    // 入れ替えるたびに一つずつ退けるので、片付けないと溜まっていく。
    sweep_retired(directory);

    if let Err(busy) = std::fs::copy(source, &destination) {
        // まだ握られている。改名なら通るので退ける。
        retire(&destination).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "{} を入れ替えられません。上書き: {busy} / 退避: {e}",
                    destination.display()
                ),
            )
        })?;
        std::fs::copy(source, &destination)?;
    }
    Ok(stopped)
}

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

    let retired = place(source, &directory, &destination)?;

    // 利用者ごとの登録が残っていると、そちらが優先されて古い DLL が
    // 使われ続ける。入れ直すたびに必ず消す。
    let cleared_per_user = registry::per_user_dll_path().is_some();
    registry::unregister_per_user_class().map_err(|e| to_io("古い利用者ごとの登録の削除", e))?;

    let path = destination.to_string_lossy().into_owned();
    registry::register_class(&path).map_err(|e| to_io("COM のクラス登録", e))?;

    // 設定画面の一覧に出す絵。DLL に資源が無いと言語名が出るだけなので、
    // `.ico` を書き出して、そちらを指す (ADR-0009)。
    let icon = write_icon(&directory);
    let icon_path = icon
        .as_ref()
        .map_or_else(|| path.clone(), |p| p.to_string_lossy().into_owned());
    profile::register_profile(&icon_path).map_err(|e| to_io("入力方式の登録", e))?;

    Ok(Installed {
        source: source.to_path_buf(),
        dll: destination,
        replaced,
        retired,
        cleared_per_user,
    })
}

/// 32 ビットの DLL を置き、32 ビット用の側に登録する。置いた場所を返す。
///
/// 入力方式の登録は 64 ビットの DLL と共通なので、ここでは COM のクラス
/// だけを登録する。使用中なら退けるのは 64 ビットのものと同じ。
pub fn install_wow32(source: &Path) -> io::Result<PathBuf> {
    let directory = wow32_dir(&install_dir()?);
    std::fs::create_dir_all(&directory)?;
    let destination = directory.join(DLL_NAME);
    place(source, &directory, &destination)?;
    let path = destination.to_string_lossy().into_owned();
    registry::register_class_in(registry::View::Wow32, &path)
        .map_err(|e| to_io("32 ビットの COM のクラス登録", e))?;
    Ok(destination)
}

/// `source` を `destination` へ写す。使用中で上書きできなければ、古いものを
/// 退けてから写す。退けたら `true`。
fn place(source: &Path, directory: &Path, destination: &Path) -> io::Result<bool> {
    // 前に退けたものを片付ける。使用中なら消せないが、それでよい。
    sweep_retired(directory);

    // 同じ場所を指しているなら写す必要はない。
    let same = std::fs::canonicalize(source)
        .ok()
        .zip(std::fs::canonicalize(destination).ok())
        .is_some_and(|(a, b)| a == b);
    if same {
        return Ok(false);
    }

    // まず上書きを試す。誰も読み込んでいなければこれで済む。
    let Err(busy) = std::fs::copy(source, destination) else {
        return Ok(false);
    };
    // 読み込まれていて上書きできない。改名なら通るので、退ける。
    retire(destination).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "{} を入れ替えられません。上書き: {busy} / 退避: {e}",
                destination.display()
            ),
        )
    })?;
    std::fs::copy(source, destination).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("{} へ写せません: {e}", destination.display()),
        )
    })?;
    Ok(true)
}

/// 設定画面へ渡す絵を書き出す。書けなければ諦める。
///
/// 絵が無くても入力はできる。ここで失敗しても導入は続ける。
fn write_icon(directory: &Path) -> Option<PathBuf> {
    // 顔の絵は SVG からビルドのときに描いてある (ADR-0024)。
    let path = directory.join(ICON_NAME);
    std::fs::write(&path, icon::FACE_ICO).ok()?;
    Some(path)
}

/// 使用中の DLL や辞書サーバを別名へ退ける。
///
/// 読み込まれていても改名はできる。掴んでいるプロセスは実体を見ており、
/// 名前を見ているわけではない。
pub(crate) fn retire(destination: &Path) -> io::Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();

    let mut retired = destination.as_os_str().to_owned();
    retired.push(format!(".{RETIRED_SUFFIX}{stamp}"));
    std::fs::rename(destination, PathBuf::from(retired))
}

/// 退けた残骸を消す。使用中なら消せないので、黙って見逃す。
///
/// TIP の DLL も辞書サーバも、入れ替えるときに使用中なら退ける
/// ([`retire`])。どちらの残骸もここで片付ける。
fn sweep_retired(directory: &Path) {
    sweep_retired_where(directory, is_ours);
}

/// TIP の DLL か辞書サーバの名前か。
fn is_ours(name: &str) -> bool {
    name == DLL_NAME || name == crate::server::SERVER_NAME
}

/// 退けた残骸のうち、元の名前が `ours` に当たるものを消す。使用中なら
/// 消せないので、黙って見逃す。
pub(crate) fn sweep_retired_where(directory: &Path, ours: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if retired_base(&entry.file_name().to_string_lossy()).is_some_and(&ours) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 中身を置く。上書きできなければ ([`retire`] の理由で) 古いものを退けて
/// から置く。
pub(crate) fn put(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    if std::fs::write(destination, bytes).is_ok() {
        return Ok(());
    }
    retire(destination)?;
    std::fs::write(destination, bytes)
}

/// [`retire`] が退けたものの名前か。`<元の名前>.old-<時刻>` の形をしている。
///
/// 元の名前まで照らす。同じ場所にある、たまたま `old-` を含むだけの
/// ファイルは消さない。
#[cfg(test)]
fn is_retired(name: &str) -> bool {
    retired_base(name).is_some_and(is_ours)
}

/// 退けたものの名前なら、元の名前を返す。
fn retired_base(name: &str) -> Option<&str> {
    let (base, stamp) = name.rsplit_once(&format!(".{RETIRED_SUFFIX}"))?;
    (!stamp.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

/// 退けたものの名前に挟む印。
const RETIRED_SUFFIX: &str = "old-";

/// 削除する。`purge` が真なら写したものも消す。
///
/// 使用中で消せないものは、再起動したときに消える予約をする。予約したら
/// `true` を返す。
///
/// 登録されていなくても失敗にしない。中途半端な状態からでも、呼べば
/// きれいになることを優先する。
pub fn uninstall(purge: bool) -> io::Result<bool> {
    let profile = profile::unregister_profile();
    let class = registry::unregister_class();
    let wow32 = registry::unregister_wow32_class();

    let mut scheduled = false;
    if purge && let Ok(directory) = install_dir() {
        // `bin` と言語モデルの置き場には、こちらが置いたものしか入って
        // いない (ADR-0031)。中身ごと消す。
        scheduled |= purge_tree(&directory);
        scheduled |= purge_tree(&crystalskk_server::paths::ranker_dir(&directory));
        // 親はインストーラの置き場も入っているかもしれない。空のときだけ
        // 消えるので、予約しても害はない。
        if let Some(parent) = directory.parent() {
            scheduled |= remove_or_schedule(parent, true);
        }
    }

    profile
        .and(class)
        .and(wow32)
        .map_err(|e| to_io("登録の解除", e))?;
    Ok(scheduled)
}

/// フォルダを中身ごと消す。消せないものは再起動のときに消える予約をする。
/// 予約したら `true`。
fn purge_tree(directory: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    let mut scheduled = false;
    for entry in entries.flatten() {
        let path = entry.path();
        scheduled |= if path.is_dir() {
            purge_tree(&path)
        } else {
            remove_or_schedule(&path, false)
        };
    }
    // 中身の予約が先に並ぶので、再起動のときには空になっている。
    scheduled | remove_or_schedule(directory, true)
}

/// 消す。使用中で消せなければ、再起動したときに消える予約をする。予約
/// したら `true`。
fn remove_or_schedule(path: &Path, directory: bool) -> bool {
    let removed = if directory {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    };
    if removed.is_ok() || !path.exists() {
        return false;
    }
    let path = HSTRING::from(path.as_os_str());
    // SAFETY: 名前は有効な文字列。行き先を渡さないのは「消す」の意。
    unsafe { MoveFileExW(&path, None, MOVEFILE_DELAY_UNTIL_REBOOT) }.is_ok()
}

/// 導入されているか調べる。
pub fn status() -> Status {
    Status {
        machine: registry::machine_dll_path().map(PathBuf::from),
        per_user: registry::per_user_dll_path().map(PathBuf::from),
        wow32: registry::wow32_dll_path().map(PathBuf::from),
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

    /// 試験ごとに固有の作業ディレクトリ。
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crystalskk-install-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作業ディレクトリを作れる");
        dir
    }

    /// 退けたファイルを数える。
    fn retired_count(directory: &Path) -> usize {
        std::fs::read_dir(directory)
            .expect("読める")
            .flatten()
            .filter(|e| is_retired(&e.file_name().to_string_lossy()))
            .count()
    }

    #[test]
    fn retiring_moves_the_dll_aside_under_a_new_name() {
        let dir = scratch("retire");
        let dll = dir.join(DLL_NAME);
        std::fs::write(&dll, "古い中身").expect("置ける");

        retire(&dll).expect("退けられる");

        assert!(!dll.exists(), "元の名前は空く");
        assert_eq!(retired_count(&dir), 1);
    }

    #[test]
    fn the_freed_name_can_be_taken_by_the_new_dll() {
        let dir = scratch("swap");
        let dll = dir.join(DLL_NAME);
        std::fs::write(&dll, "古い中身").expect("置ける");

        retire(&dll).expect("退けられる");
        std::fs::write(&dll, "新しい中身").expect("同じ名前で置ける");

        assert_eq!(std::fs::read_to_string(&dll).expect("読める"), "新しい中身");
    }

    #[test]
    fn sweeping_removes_what_was_retired_but_leaves_the_dll() {
        let dir = scratch("sweep");
        let dll = dir.join(DLL_NAME);
        std::fs::write(&dll, "中身").expect("置ける");
        retire(&dll).expect("退けられる");
        std::fs::write(&dll, "新しい中身").expect("置ける");
        assert_eq!(retired_count(&dir), 1);

        sweep_retired(&dir);

        assert_eq!(retired_count(&dir), 0);
        assert!(dll.exists(), "使っているものは消さない");
    }

    #[test]
    fn sweeping_removes_retired_servers_but_leaves_the_server() {
        // 辞書サーバも入れ替えのたびに退けられる。**片付けないと溜まる。**
        let dir = scratch("sweep-server");
        let server = dir.join(crate::server::SERVER_NAME);
        std::fs::write(dir.join("crystalskk-server.exe.old-1790128010"), "古い").expect("置ける");
        std::fs::write(dir.join("crystalskk-server.exe.old-1790128145"), "古い").expect("置ける");
        std::fs::write(&server, "新しい").expect("置ける");
        assert_eq!(retired_count(&dir), 2);

        sweep_retired(&dir);

        assert_eq!(retired_count(&dir), 0);
        assert!(server.exists(), "使っているものは消さない");
    }

    #[test]
    fn sweeping_takes_only_the_names_it_is_told() {
        // 言語モデルの置き場では、モデル一式の名前だけを片付ける。
        let dir = scratch("sweep-where");
        std::fs::write(dir.join("model.gguf.old-1790128010"), "古い").expect("置ける");
        std::fs::write(dir.join("notes.txt.old-1790128010"), "他人").expect("置ける");

        sweep_retired_where(&dir, |base| base == "model.gguf");

        assert!(!dir.join("model.gguf.old-1790128010").exists());
        assert!(dir.join("notes.txt.old-1790128010").exists());
    }

    #[test]
    fn putting_replaces_what_is_there() {
        let dir = scratch("put");
        let path = dir.join("model.gguf");
        std::fs::write(&path, "古い").expect("置ける");

        put(&path, "新しい".as_bytes()).expect("置ける");

        assert_eq!(std::fs::read_to_string(&path).expect("読める"), "新しい");
    }

    #[test]
    fn sweeping_leaves_files_that_only_look_old() {
        let dir = scratch("sweep-strangers");
        for name in [
            "notes.old-1.txt",
            "crystalskk.ico",
            "crystalskk_tip.dllold-1",
            "log.on",
        ] {
            std::fs::write(dir.join(name), "中身").expect("置ける");
        }

        sweep_retired(&dir);

        assert_eq!(
            std::fs::read_dir(&dir).expect("読める").count(),
            4,
            "どれも消さない"
        );
    }

    #[test]
    fn what_retire_leaves_is_recognised_as_retired() {
        assert!(is_retired("crystalskk_tip.dll.old-1790147371"));
        assert!(is_retired("crystalskk-server.exe.old-1790168724"));
        assert!(!is_retired("crystalskk_tip.dll"));
        assert!(!is_retired("crystalskk-server.exe"));
        assert!(!is_retired("other.dll.old-1"));
    }

    #[test]
    fn sweeping_an_empty_directory_does_nothing() {
        let dir = scratch("sweep-empty");
        sweep_retired(&dir);
        assert_eq!(retired_count(&dir), 0);
    }

    #[test]
    fn installing_a_missing_file_fails_before_touching_the_registry() {
        let error = install(Path::new("存在しない.dll")).expect_err("失敗する");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
