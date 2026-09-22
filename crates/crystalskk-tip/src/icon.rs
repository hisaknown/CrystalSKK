//! 入力モードのアイコンを、その場で描いて作る。
//!
//! トレイの表示は絵がないと出ない。文字を返しても描かれない。
//!
//! **絵柄は仮のものである。** 地の四角にモードの文字を白抜きするだけで、
//! 明るい背景でも暗い背景でも読めることだけを狙っている。差し替える
//! 前提で、描き方は `draw` 一箇所に閉じてある。
//!
//! # 本物の絵を入れるとき
//!
//! 描いた絵ではなく、用意した絵を使う段になったら二つ道がある。
//!
//! - **`.ico` を `include_bytes!` で抱え、`CreateIconFromResourceEx` で
//!   `HICON` にする。** リソースコンパイラは要らず、ここの `render` を
//!   差し替えるだけで済む。言語バーとトレイにはこれで足りる。
//! - **PE の資源として埋め込む。** 設定画面の一覧に出る絵
//!   (`RegisterProfile` に渡す「アイコンのあるファイルと番号」) は、
//!   本物の資源でなければ読まれない。こちらが要るならリソース
//!   コンパイラか、`.res` を自前で組み立てる仕掛けが要る。
//!
//! 一つ目で困らないうちは、二つ目に手を出さなくてよい。
//!
//! # `.ico` ファイルも書ける
//!
//! 設定画面の一覧に出る絵は `RegisterProfile` に「絵のあるファイルと
//! 番号」を渡して指す。**そのファイルは `.ico` でよい。** DLL の資源で
//! ある必要はない。そこで、導入のときに描いた絵を `.ico` として書き出し、
//! それを指す。リソースコンパイラは要らない。

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC, CreateDIBSection,
    CreateFontW, CreateSolidBrush, DEFAULT_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_SEMIBOLD,
    FillRect, GdiFlush, GetDC, GetDeviceCaps, HBITMAP, HDC, HFONT, LOGPIXELSY, OUT_DEFAULT_PRECIS,
    ReleaseDC, SHIFTJIS_CHARSET, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::{Result, w};

/// 基準の大きさ。拡大率に合わせて伸ばす。
const BASE_SIZE: i32 = 16;

/// 標準の拡大率での画素密度。
const BASE_DPI: i32 = 96;

/// これ以上は大きくしない。
const MAX_SIZE: i32 = 64;

/// 地の色。暗い紺。明るい背景でも沈んで見える。
const BACKGROUND: COLORREF = COLORREF(0x00_55_3A_2B);

/// 文字の色。
const FOREGROUND: COLORREF = COLORREF(0x00_FF_FF_FF);

/// モードの文字を描いたアイコンを作る。
///
/// 返したアイコンは呼び出し側が [`windows::Win32::UI::WindowsAndMessaging::DestroyIcon`]
/// で解放する。言語バーはそう扱う。
pub fn render(label: &str) -> Result<HICON> {
    let size = icon_size();

    // SAFETY: 以下は GDI の定める手順どおりで、作ったものはすべて
    // この関数の中で後始末する。
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));

        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = create_surface(memory, size, &mut bits);
        let color = match color {
            Ok(color) => color,
            Err(e) => {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return Err(e);
            }
        };

        let previous = SelectObject(memory, color.into());
        draw(memory, size, label);
        SelectObject(memory, previous);

        // GDI の描画は溜められてから実行される。画素へ直接触る前に
        // 吐き出させないと、まだ描かれていないものを読むことになる。
        let _ = GdiFlush();

        // GDI は透過情報を書かないので、全面を不透明にする。
        fill_alpha(bits, size);

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
        match &icon {
            Ok(_) => crate::log::write(&format!("アイコンを作った ({size} 画素, 「{label}」)")),
            Err(e) => crate::log::write(&format!("アイコンを作れなかった: {}", e.message())),
        }

        // アイコンは中身を写して作られるので、こちらの絵は捨ててよい。
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);

        icon
    }
}

/// 描いた絵を `.ico` の中身にする。
///
/// 設定画面へ渡すのはファイルなので、そこへ書き出せる形が要る。
pub fn ico_bytes(label: &str, size: i32) -> Option<Vec<u8>> {
    let pixels = draw_pixels(label, size)?;
    Some(encode_ico(&pixels, size))
}

/// 描いた結果の画素を取り出す。上の行が先、一画素 32 ビット。
fn draw_pixels(label: &str, size: i32) -> Option<Vec<u32>> {
    // SAFETY: GDI の手順どおりで、作ったものはこの関数の中で後始末する。
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));

        let mut bits: *mut c_void = std::ptr::null_mut();
        let surface = create_surface(memory, size, &mut bits).ok();

        let pixels = surface.and_then(|surface| {
            let previous = SelectObject(memory, surface.into());
            draw(memory, size, label);
            SelectObject(memory, previous);
            let _ = GdiFlush();
            fill_alpha(bits, size);

            let taken = (!bits.is_null()).then(|| {
                let count = (size * size).max(0) as usize;
                std::slice::from_raw_parts(bits.cast::<u32>(), count).to_vec()
            });
            let _ = DeleteObject(surface.into());
            taken
        });

        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
    }
}

/// 画素を `.ico` の並びにする。
///
/// `.ico` の中の絵は下の行が先で、高さは覆いの分を足して二倍に書く。
/// 覆いは使わないので全て 0 にする。
fn encode_ico(pixels: &[u32], size: i32) -> Vec<u8> {
    let size = size.max(0);
    let width = size as usize;
    // 覆いは 4 バイト境界に揃える。
    let mask_row = width.div_ceil(32) * 4;
    let image_len = HEADER_LEN + width * width * 4 + mask_row * width;

    let mut out = Vec::with_capacity(DIR_LEN + ENTRY_LEN + image_len);

    // 目録。
    out.extend_from_slice(&0u16.to_le_bytes()); // 予約
    out.extend_from_slice(&1u16.to_le_bytes()); // 種別: アイコン
    out.extend_from_slice(&1u16.to_le_bytes()); // 枚数

    // 一枚分の見出し。256 画素は 0 で表す決まり。
    let dimension = u8::try_from(size).unwrap_or(0);
    out.push(dimension);
    out.push(dimension);
    out.push(0); // 色数: 32 ビットなので 0
    out.push(0); // 予約
    out.extend_from_slice(&1u16.to_le_bytes()); // 面数
    out.extend_from_slice(&32u16.to_le_bytes()); // ビット数
    out.extend_from_slice(&(image_len as u32).to_le_bytes());
    out.extend_from_slice(&((DIR_LEN + ENTRY_LEN) as u32).to_le_bytes());

    // 絵の見出し。高さは絵と覆いを合わせた分。
    out.extend_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(size * 2).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 圧縮なし
    out.extend_from_slice(&0u32.to_le_bytes()); // 大きさは省略してよい
    out.extend_from_slice(&[0u8; 16]); // 解像度と色表の欄

    // 絵。下の行から書く。
    for row in (0..width).rev() {
        for column in 0..width {
            let pixel = pixels.get(row * width + column).copied().unwrap_or(0);
            out.extend_from_slice(&pixel.to_le_bytes());
        }
    }

    // 覆い。全面を「隠さない」。
    out.extend_from_slice(&vec![0u8; mask_row * width]);

    out
}

/// 目録の大きさ。
const DIR_LEN: usize = 6;

/// 一枚分の見出しの大きさ。
const ENTRY_LEN: usize = 16;

/// 絵の見出しの大きさ。
const HEADER_LEN: usize = 40;

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

/// 地を塗り、モードの文字を中央に置く。
///
/// **ここが絵柄のすべてである。** 差し替えるときはこの関数だけを書き換える。
///
/// # Safety
///
/// `dc` に描き込む面が選ばれていること。
unsafe fn draw(dc: HDC, size: i32, label: &str) {
    let area = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };

    // SAFETY: 呼び出し側の約束による。作った道具はここで捨てる。
    unsafe {
        let background = CreateSolidBrush(BACKGROUND);
        FillRect(dc, &area, background);
        let _ = DeleteObject(background.into());

        let font = mode_font(size);
        let previous = SelectObject(dc, font.into());

        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, FOREGROUND);

        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut area = area;
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP,
        );

        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
    }
}

/// モードの文字に使う字体。
///
/// 仮名を含むので、仮名を持つ字体を頼む。無ければ Windows が similar な
/// ものを選ぶ。
fn mode_font(size: i32) -> HFONT {
    // SAFETY: 大きさと種別を渡して字体を頼むだけ。
    unsafe {
        CreateFontW(
            // 四角いっぱいだと窮屈なので少し縮める。
            -(size * 3 / 4),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            SHIFTJIS_CHARSET,
            OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            FF_DONTCARE.0.into(),
            w!("Yu Gothic UI"),
        )
    }
}

/// 覆いに要るバイト数。
///
/// 単色の絵は各行が 2 バイト境界に揃う。
fn mask_len(size: i32) -> usize {
    let size = size.max(0) as usize;
    let bytes_per_row = size.div_ceil(16) * 2;
    bytes_per_row * size
}

/// 全面を不透明にする。
///
/// GDI の描画は透過の情報を触らないので、そのままでは全部が透明のまま
/// 扱われて何も見えない。
///
/// # Safety
///
/// `bits` が `size * size` 個の 32 ビット画素を指していること。
unsafe fn fill_alpha(bits: *mut c_void, size: i32) {
    if bits.is_null() {
        crate::log::write("描き込む面を取れなかった");
        return;
    }
    let count = (size * size).max(0) as usize;
    // SAFETY: 呼び出し側の約束による。面は 32 ビット画素で作ってある。
    let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), count) };
    for pixel in pixels {
        *pixel |= 0xFF00_0000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ico_starts_with_a_directory_for_one_image() {
        let pixels = vec![0xFF00_0000u32; 32 * 32];
        let ico = encode_ico(&pixels, 32);

        assert_eq!(&ico[0..2], &[0, 0], "予約の欄");
        assert_eq!(&ico[2..4], &[1, 0], "種別はアイコン");
        assert_eq!(&ico[4..6], &[1, 0], "一枚だけ");
        assert_eq!(ico[6], 32, "幅");
        assert_eq!(ico[7], 32, "高さ");
        assert_eq!(&ico[10..12], &[1, 0], "面数");
        assert_eq!(&ico[12..14], &[32, 0], "一画素 32 ビット");
    }

    #[test]
    fn the_ico_is_exactly_as_long_as_it_says() {
        let pixels = vec![0xFF00_0000u32; 32 * 32];
        let ico = encode_ico(&pixels, 32);

        let declared = u32::from_le_bytes(ico[14..18].try_into().expect("4 バイト")) as usize;
        let offset = u32::from_le_bytes(ico[18..22].try_into().expect("4 バイト")) as usize;
        assert_eq!(offset, DIR_LEN + ENTRY_LEN);
        assert_eq!(ico.len(), offset + declared, "宣言した長さと実際が合う");
    }

    #[test]
    fn the_ico_header_doubles_the_height_for_the_mask() {
        let pixels = vec![0u32; 16 * 16];
        let ico = encode_ico(&pixels, 16);
        let header = DIR_LEN + ENTRY_LEN;

        let width = i32::from_le_bytes(ico[header + 4..header + 8].try_into().expect("4 バイト"));
        let height = i32::from_le_bytes(ico[header + 8..header + 12].try_into().expect("4 バイト"));
        assert_eq!(width, 16);
        assert_eq!(height, 32, "絵と覆いの分");
    }

    #[test]
    fn the_ico_rows_are_written_bottom_up() {
        // 一行目だけ印を付けて、最後の行に来ることを見る。
        let mut pixels = vec![0u32; 2 * 2];
        pixels[0] = 0xDEAD_BEEF;
        let ico = encode_ico(&pixels, 2);

        let image = DIR_LEN + ENTRY_LEN + HEADER_LEN;
        // 下から書くので、上の行は後ろに来る。
        let last_row = image + 2 * 4;
        assert_eq!(
            u32::from_le_bytes(ico[last_row..last_row + 4].try_into().expect("4 バイト")),
            0xDEAD_BEEF
        );
    }

    #[test]
    fn the_mask_is_word_aligned_per_row() {
        // 16 画素で 2 バイト、17 画素で 4 バイト。
        assert_eq!(mask_len(16), 2 * 16);
        assert_eq!(mask_len(17), 4 * 17);
        assert_eq!(mask_len(32), 4 * 32);
    }

    #[test]
    fn an_empty_icon_needs_no_mask() {
        assert_eq!(mask_len(0), 0);
    }
}
