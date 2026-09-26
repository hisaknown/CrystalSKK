//! タスクバーの明るさに合わせる。
//!
//! 入力モードの絵は単色で描く。Windows 標準の IME と同じく、タスクバーが
//! 明るければ黒、暗ければ白にする。**IME がテーマ別の絵を渡す口は無い**
//! ので、こちらでテーマを読み、変わったら描き直させる。
//!
//! 見るのはアプリの明るさ (`AppsUseLightTheme`) ではなく、**タスクバーの
//! 明るさ** (`SystemUsesLightTheme`) である。絵が載るのはタスクバーで、
//! 二つは別々に選べる。

use windows::Win32::Graphics::Gdi::{
    COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
    SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{PCWSTR, w};

/// タスクバーの明るさ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    /// いまのタスクバーの明るさ。
    ///
    /// 読めなければ暗いほうとする。Windows 10 の 1903 からの既定がそれで、
    /// 値が無いのはそれより古いときである (そのころのタスクバーも暗い)。
    pub fn current() -> Self {
        read(w!("SystemUsesLightTheme"), Self::Dark)
    }

    /// いまのアプリの明るさ。カーソルのそばに出す窓は、こちらに合わせる。
    ///
    /// 窓が載るのはタスクバーではなくアプリの上である。二つは別々に選べる。
    /// 読めなければ明るいほうとする (アプリの既定)。
    pub fn apps() -> Self {
        read(w!("AppsUseLightTheme"), Self::Light)
    }

    /// 絵を描く色。`0xRRGGBB`。
    ///
    /// **明るいタスクバーには黒、暗いタスクバーには白。** 色を設定で
    /// 選べるようにするなら、ここが差し替わる。
    pub fn ink(self) -> u32 {
        match self {
            Self::Light => 0x00_00_00,
            Self::Dark => 0xFF_FF_FF,
        }
    }
}

/// `Personalize` の下の明暗の値を読む。1 なら明るい。
fn read(name: PCWSTR, fallback: Theme) -> Theme {
    match personalize(name) {
        None => fallback,
        Some(0) => Theme::Dark,
        Some(_) => Theme::Light,
    }
}

/// 「透明効果」が入っているか。読めなければ入っているとする (Windows の既定)。
pub(crate) fn transparency() -> bool {
    personalize(w!("EnableTransparency")) != Some(0)
}

/// `Personalize` の下の値を読む。
fn personalize(name: PCWSTR) -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
    // SAFETY: 書き込み先はこの関数の変数で、大きさも渡している。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    status.is_ok().then_some(value)
}

/// アプリの上に出す小窓 (候補の窓、カーソルのそばの窓) の色。どれも `0xRRGGBB`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub text: u32,
    pub border: u32,
    /// 選んでいる行 (反転の帯) の地。
    pub selected_background: u32,
    /// 選んでいる行の文字。
    pub selected_text: u32,
    /// 押すキーを囲む枠と、選んでいる行の印。
    pub key: u32,
    /// 透かした地に重ねる地の色の濃さ (百分率)。
    pub backdrop_opacity: u8,
}

impl Palette {
    /// 設定の色を、いまの明るさに合わせて解く (ADR-0026)。
    ///
    /// - **ハイコントラストなら、設定にかかわらずシステムの色に従う。**
    ///   利用者が選んだ配色を上書きしてはならない。
    /// - `theme` が `auto` なら、アプリの明るさで組を選ぶ。
    /// - `"system"` は役目ごとの Windows の標準の色、`"accent"` はアクセント
    ///   カラー。
    pub fn resolve(colors: &crystalskk_settings::Colors) -> Self {
        use crystalskk_settings::ThemeChoice;
        if high_contrast() {
            return Self::system();
        }
        let dark = match colors.theme {
            ThemeChoice::Auto => Theme::apps() == Theme::Dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        };
        let set = if dark { &colors.dark } else { &colors.light };
        let background = pick(set.background, COLOR_WINDOW);
        let text = pick(set.text, COLOR_WINDOWTEXT);
        // 選んでいる行の "system" は、`COLOR_HIGHLIGHT` ではなく地と文字から
        // 作る。**強調の色で塗ると帯が重く、同じ色の印も埋もれる。** 地に
        // 文字の色を少し混ぜれば、明るい窓でも暗い窓でも控えめな帯になる。
        let selected_background = match set.selected_background {
            crystalskk_settings::Color::System => blend(text, background, BAND_TINT),
            color => pick(color, COLOR_HIGHLIGHT),
        };
        let selected_text = match set.selected_text {
            crystalskk_settings::Color::System => text,
            color => pick(color, COLOR_HIGHLIGHTTEXT),
        };
        Self {
            background,
            text,
            border: pick(set.border, COLOR_HIGHLIGHT),
            selected_background,
            selected_text,
            key: pick(set.key, COLOR_HIGHLIGHT),
            backdrop_opacity: colors.backdrop_opacity.min(100),
        }
    }

    /// Windows の標準の色だけの組。
    ///
    /// 設定をまだ受け取っていないとき (設定が読めないことを知らせる窓など)
    /// に使う。
    pub fn system() -> Self {
        Self {
            background: system(COLOR_WINDOW),
            text: system(COLOR_WINDOWTEXT),
            border: system(COLOR_HIGHLIGHT),
            selected_background: system(COLOR_HIGHLIGHT),
            selected_text: system(COLOR_HIGHLIGHTTEXT),
            key: system(COLOR_HIGHLIGHT),
            // 標準の色だけの組は透かさない。
            backdrop_opacity: 100,
        }
    }
}

/// 選んでいる行の帯に混ぜる文字の色の割合。256 分の。
const BAND_TINT: u32 = 26;

/// `a` を `weight` / 256 だけ `b` に混ぜる。
fn blend(a: u32, b: u32, weight: u32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((a * weight + b * (256 - weight)) / 256) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// 設定の色を解く。`"system"` なら `role` の標準の色。
fn pick(color: crystalskk_settings::Color, role: SYS_COLOR_INDEX) -> u32 {
    use crystalskk_settings::Color;
    match color {
        Color::System => system(role),
        // アクセントカラーが読めない (古い Windows) なら、強調の色で代える。
        Color::Accent => accent().unwrap_or_else(|| system(COLOR_HIGHLIGHT)),
        Color::Rgb(rgb) => rgb & 0x00FF_FFFF,
    }
}

/// システムの色を `0xRRGGBB` で。
fn system(index: SYS_COLOR_INDEX) -> u32 {
    // SAFETY: 番号を渡して色を受け取るだけ。
    let c = unsafe { GetSysColor(index) };
    ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

/// Windows のアクセントカラー。`0xRRGGBB`。
///
/// 「個人用設定 → 色」で選ぶ色で、`DWM\AccentColor` に `0xAABBGGRR` で
/// 置かれている。**`COLOR_HIGHLIGHT` はこれに追従しない。**
fn accent() -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
    // SAFETY: 書き込み先はこの関数の変数で、大きさも渡している。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    status.is_ok().then(|| {
        let (r, g, b) = (value & 0xFF, (value >> 8) & 0xFF, (value >> 16) & 0xFF);
        (r << 16) | (g << 8) | b
    })
}

/// ハイコントラストの配色が選ばれているか。
pub(crate) fn high_contrast() -> bool {
    let mut info = HIGHCONTRASTW {
        cbSize: u32::try_from(size_of::<HIGHCONTRASTW>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: 大きさを告げた構造体へ書かせる。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some(std::ptr::from_mut::<HIGHCONTRASTW>(&mut info).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ink_stands_out_from_the_taskbar() {
        assert_eq!(Theme::Light.ink(), 0x00_00_00, "明るいタスクバーには黒");
        assert_eq!(Theme::Dark.ink(), 0xFF_FF_FF, "暗いタスクバーには白");
    }

    #[test]
    fn the_theme_can_be_read() {
        // どちらになるかは機械次第。読めて、落ちないこと。
        let _ = Theme::current();
        let _ = Theme::apps();
        let _ = Palette::system();
        let _ = accent();
    }

    #[test]
    fn the_band_is_the_ground_tinted_by_the_text() {
        let light = blend(0x00_00_00, 0xFF_FF_FF, BAND_TINT);
        let dark = blend(0xFF_FF_FF, 0x2B_2B_2B, BAND_TINT);
        assert!(
            light < 0xFF_FF_FF && light > 0xD0_D0_D0,
            "明るい窓では薄いグレー: {light:06X}"
        );
        assert!(
            dark > 0x2B_2B_2B && dark < 0x50_50_50,
            "暗い窓では少し明るいグレー: {dark:06X}"
        );
    }

    #[test]
    fn written_colours_are_used_as_they_are() {
        use crystalskk_settings::Color;
        assert_eq!(pick(Color::Rgb(0x12_34_56), COLOR_WINDOW), 0x12_34_56);
        assert_eq!(pick(Color::System, COLOR_WINDOW), system(COLOR_WINDOW));
    }
}
