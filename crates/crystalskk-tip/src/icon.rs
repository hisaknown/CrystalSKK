//! アイコン。
//!
//! 絵の正本は `assets/icons/` の SVG で、ビルドのときに描いて埋め込んである
//! (ADR-0024, `build.rs`)。
//!
//! - **入力モードの絵** (トレイ): 濃さだけを持っており、ここで色を付けて
//!   `HICON` にする。色はタスクバーの明るさに合わせる ([`crate::theme`])。
//!   地は透明で、Windows 標準の IME と同じく単色で出る。
//! - **顔の絵** (設定画面の一覧): 色付きの `.ico` をそのまま持っている。
//!   導入のときにファイルとして書き出し、`RegisterProfile` に渡す
//!   (ADR-0009)。一覧の絵は差し替える口が無いので、テーマに追従しない。

use std::ffi::c_void;

use crystalskk_core::InputMode;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDeviceCaps, HBITMAP, HDC, LOGPIXELSY,
    ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::Result;

/// ビルドのときに描いた絵。
mod drawn {
    /// 大きさごとの濃さ。一画素 1 バイト、上の行が先。小さい順。
    pub type Mode = &'static [(u32, &'static [u8])];

    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// 顔の絵。色付きの `.ico`。導入のときに書き出す。
pub const FACE_ICO: &[u8] = drawn::FACE_ICO;

/// 基準の大きさ。拡大率に合わせて伸ばす。
const BASE_SIZE: i32 = 16;

/// 標準の拡大率での画素密度。
const BASE_DPI: i32 = 96;

/// これ以上は大きくしない。
const MAX_SIZE: i32 = 64;

/// 入力モードの絵。`None` は入力方式が切。
fn mode_drawing(mode: Option<InputMode>) -> drawn::Mode {
    match mode {
        Some(InputMode::Hiragana) => drawn::HIRAGANA,
        Some(InputMode::Katakana) => drawn::KATAKANA,
        Some(InputMode::HalfKatakana) => drawn::HALFWIDTH_KATAKANA,
        Some(InputMode::Ascii) => drawn::ASCII,
        Some(InputMode::FullAscii) => drawn::FULLWIDTH_ASCII,
        None => drawn::OFF,
    }
}

/// 欲しい大きさに合う絵を選ぶ。
///
/// 同じ大きさがあればそれ。無ければ、それより大きいうちでいちばん小さい
/// もの (**縮めるほうが、伸ばすより崩れにくい**)。どれより大きければ、
/// いちばん大きいもの。
fn pick(drawing: drawn::Mode, wanted: u32) -> (u32, &'static [u8]) {
    drawing
        .iter()
        .copied()
        .find(|(size, _)| *size >= wanted)
        .or_else(|| drawing.last().copied())
        .expect("どのモードの絵も一枚以上ある")
}

/// 入力モードのアイコンを作る。**単色で、地は透明。**
///
/// 色 (`0xRRGGBB`) はタスクバーの明るさに合わせて渡す
/// ([`crate::theme::Theme::ink`])。
///
/// 返したアイコンは呼び出し側が [`windows::Win32::UI::WindowsAndMessaging::DestroyIcon`]
/// で解放する。言語バーはそう扱う。
pub fn render(mode: Option<InputMode>, ink: u32) -> Result<HICON> {
    let wanted = u32::try_from(icon_size()).unwrap_or(16);
    let (size, coverage) = pick(mode_drawing(mode), wanted);
    let pixels = tint(coverage, ink);
    let icon = icon_from_pixels(&pixels, i32::try_from(size).unwrap_or(16));
    match &icon {
        Ok(_) => crate::log::trace(&format!("アイコンを作った ({size} 画素, {mode:?})")),
        Err(e) => crate::log::error(&format!("アイコンを作れなかった: {}", e.message())),
    }
    icon
}

/// 濃さに色を付ける。一画素 `0xAARRGGBB`。
fn tint(coverage: &[u8], ink: u32) -> Vec<u32> {
    coverage
        .iter()
        .map(|alpha| (u32::from(*alpha) << 24) | (ink & 0x00FF_FFFF))
        .collect()
}

/// 画素から `HICON` を作る。上の行が先、一画素 `0xAARRGGBB`。
fn icon_from_pixels(pixels: &[u32], size: i32) -> Result<HICON> {
    // SAFETY: GDI の手順どおりで、作ったものはすべてこの関数の中で後始末する。
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = create_surface(memory, size, &mut bits);
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        let color = color?;

        if !bits.is_null() {
            let count = (size * size).max(0) as usize;
            let target = std::slice::from_raw_parts_mut(bits.cast::<u32>(), count);
            for (slot, pixel) in target.iter_mut().zip(pixels) {
                *slot = *pixel;
            }
        }

        // 覆い。32 ビットの色を使うので全面を「隠さない」= 0 にする。
        //
        // `CreateBitmap` に中身を渡さないと**中身は不定**になる。覆いが
        // でたらめだとアイコンはまだらに、あるいは丸ごと透明になる。
        let mask_bits = vec![0u8; mask_len(size)];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));

        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);

        // アイコンは中身を写して作られるので、こちらの絵は捨ててよい。
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// 拡大率に合わせた一辺の長さ。
fn icon_size() -> i32 {
    // SAFETY: 画面の DC を借りて問い合わせ、すぐ返す。
    let dpi = unsafe {
        let screen = GetDC(None);
        let dpi = GetDeviceCaps(Some(screen), LOGPIXELSY);
        ReleaseDC(None, screen);
        dpi
    };
    let dpi = if dpi > 0 { dpi } else { BASE_DPI };
    (BASE_SIZE * dpi / BASE_DPI).clamp(BASE_SIZE, MAX_SIZE)
}

/// 描き込む面を作る。画素へ直接触れるよう、上下を正した 32 ビットの面にする。
///
/// # Safety
///
/// `bits` は書き込める場所を指していること。
unsafe fn create_surface(dc: HDC, size: i32, bits: *mut *mut c_void) -> Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
            biWidth: size,
            // 負にすると上が先になる。画素をそのまま数えられる。
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };

    // SAFETY: 呼び出し側の約束による。
    unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, bits, None, 0) }
}

/// 覆いに要るバイト数。
///
/// 一行は 2 バイト境界に揃える (`CreateBitmap` の決まり)。
fn mask_len(size: i32) -> usize {
    let size = size.max(0) as usize;
    let bytes_per_row = size.div_ceil(16) * 2;
    bytes_per_row * size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_a_drawing_for_the_usual_scales() {
        // 100% から 300% まで。**近い大きさを縮めると滲む。**
        for mode in [
            Some(InputMode::Hiragana),
            Some(InputMode::Katakana),
            Some(InputMode::HalfKatakana),
            Some(InputMode::Ascii),
            Some(InputMode::FullAscii),
            None,
        ] {
            for wanted in [16, 20, 24, 28, 32, 40, 48] {
                let (size, coverage) = pick(mode_drawing(mode), wanted);
                assert_eq!(size, wanted, "{mode:?} の {wanted} 画素");
                assert_eq!(coverage.len(), (size * size) as usize);
            }
        }
    }

    #[test]
    fn a_drawing_has_ink_and_a_clear_ground() {
        // 地は透明、絵の部分だけに濃さがある。**全面が塗られていたら、
        // タスクバーに四角が出る。**
        let (size, coverage) = pick(drawn::HIRAGANA, 32);
        assert_eq!(coverage[0], 0, "左上は透明");
        assert_eq!(coverage[(size * size - 1) as usize], 0, "右下は透明");
        assert!(coverage.contains(&255), "濃いところがある");
        assert!(coverage.iter().any(|a| *a > 0 && *a < 255), "縁はなめらか");
    }

    #[test]
    fn an_odd_size_takes_the_next_larger_drawing() {
        assert_eq!(pick(drawn::HIRAGANA, 17).0, 20);
        assert_eq!(pick(drawn::HIRAGANA, 500).0, 64, "大きすぎれば最大");
    }

    #[test]
    fn the_ink_colours_every_pixel_and_keeps_the_coverage() {
        let pixels = tint(&[0, 128, 255], 0x12_34_56);
        assert_eq!(pixels, [0x0012_3456, 0x8012_3456, 0xFF12_3456]);
    }

    #[test]
    fn the_face_is_an_ico() {
        assert_eq!(&FACE_ICO[..4], &[0, 0, 1, 0], "アイコンの目録");
        let count = u16::from_le_bytes([FACE_ICO[4], FACE_ICO[5]]);
        assert!(count >= 4, "大きさ違いが何枚もある");
    }

    #[test]
    fn the_mask_is_word_aligned_per_row() {
        // 16 画素で 2 バイト、17 画素で 4 バイト。
        assert_eq!(mask_len(16), 2 * 16);
        assert_eq!(mask_len(17), 4 * 17);
        assert_eq!(mask_len(32), 4 * 32);
        assert_eq!(mask_len(0), 0);
    }
}
