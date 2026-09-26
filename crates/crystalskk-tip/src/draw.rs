//! Direct2D と DirectWrite で小窓を描くための下ごしらえ。
//!
//! GDI では小さな角の丸みがなめらかにならず、文字も粗い。**半透明も扱えない**
//! ので、窓の地を透かす (Acrylic) こともできない。そこで描くのは Direct2D に
//! 任せる。
//!
//! 座標はすべて DIP (100% のときの画素) で書き、拡大率は描く先に教える。
//! 窓の実際の大きさだけが画素である ([`pixels`])。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{HSTRING, w};

use crate::dpi;
use crate::log;

thread_local! {
    /// Direct2D の工場。入力スレッドごとに一つ持つ (一つのスレッドでしか使わない)。
    static D2D: Option<ID2D1Factory> = {
        // SAFETY: 工場を作るだけ。
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
            .inspect_err(|e| log::error(&format!("Direct2D の工場を作れなかった: {}", e.message())))
            .ok()
    };
    /// DirectWrite の工場。
    static DWRITE: Option<IDWriteFactory> = {
        // SAFETY: 工場を作るだけ。
        unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .inspect_err(|e| log::error(&format!("DirectWrite の工場を作れなかった: {}", e.message())))
            .ok()
    };
    /// 設定の大きさと書体。設定を受け取るたびに差し替える ([`configure`])。
    static LOOK: RefCell<Look> = RefCell::new(Look::default());
    /// 描く先。窓の画面へ、描くたびに結びつけ直して使い回す。
    static TARGET: RefCell<Option<ID2D1DCRenderTarget>> = const { RefCell::new(None) };
}

/// 設定の大きさと書体。
#[derive(Debug, Clone)]
struct Look {
    /// 大きさ (百分率)。
    scale: u16,
    /// 書体の名前。`None` は Windows のメッセージの書体。
    font: Option<String>,
    /// 文字の大きさ (ポイント)。`None` は Windows のメッセージの書体の大きさ。
    font_size: Option<u16>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            scale: 100,
            font: None,
            font_size: None,
        }
    }
}

/// 設定の大きさと書体を覚える。以後に出す窓から効く。
pub fn configure(popup: &crystalskk_settings::Popup) {
    LOOK.with(|look| {
        *look.borrow_mut() = Look {
            scale: popup.scale,
            font: popup.font.clone(),
            font_size: popup.font_size,
        };
    });
}

/// 画面座標の `point` に窓を出すときの、描く拡大率。
///
/// モニターの拡大率 ([`dpi::at`]) に、設定の大きさを掛け合わせる。拡大率ごと
/// 変えるので、文字も余白も絵もまとめて大きくなり、釣り合いが崩れない。
pub fn dpi_at(point: POINT) -> u32 {
    let scale = LOOK.with(|look| u32::from(look.borrow().scale));
    (dpi::at(point) * scale / 100).max(1)
}

/// `0xRRGGBB` を Direct2D の色にする。
pub fn color(rgb: u32) -> D2D1_COLOR_F {
    let channel = |shift: u32| f32::from(u8::try_from((rgb >> shift) & 0xFF).unwrap_or(0)) / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// 窓の地の色。
///
/// 透かしているなら、地の色を `opacity` (百分率) の濃さで重ね、DWM の地を
/// 少しだけ見せる。**透かしたままでは、後ろが明るいと暗い窓の文字が読めない。**
pub fn ground(rgb: u32, backdrop: bool, opacity: u8) -> D2D1_COLOR_F {
    let solid = color(rgb);
    if backdrop {
        D2D1_COLOR_F {
            a: f32::from(opacity.min(100)) / 100.0,
            ..solid
        }
    } else {
        solid
    }
}

/// DIP の長さを、`dpi` の画素数にする。切り上げる。
pub fn pixels(dip: f32, dpi: u32) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let px = (dip * dpi as f32 / dpi::BASE as f32).ceil() as i32;
    px
}

/// 案内の書体。システムのメッセージの書体に従う。
///
/// 自前で選ぶと、利用者が大きさを変えていても追随できない。
#[derive(Debug, Clone)]
pub struct Font {
    family: HSTRING,
    /// 大きさ (DIP)。
    size: f32,
    weight: i32,
}

impl Font {
    /// 案内の書体。設定に書体や大きさがあればそれ、無ければシステムの
    /// メッセージの書体。それも読めなければ Yu Gothic UI の 9pt。
    pub fn message() -> Self {
        let system = dpi::message_font_spec().map_or_else(
            || Self {
                family: HSTRING::from("Yu Gothic UI"),
                size: 12.0,
                weight: 400,
            },
            |(family, size, weight)| Self {
                family: HSTRING::from(family),
                size,
                weight,
            },
        );
        LOOK.with(|look| {
            let look = look.borrow();
            Self {
                family: look
                    .font
                    .as_deref()
                    .map_or(system.family.clone(), HSTRING::from),
                // ポイントを DIP にする。1 ポイントは 1/72 インチ、1 DIP は 1/96 インチ。
                size: look
                    .font_size
                    .map_or(system.size, |pt| f32::from(pt) * 96.0 / 72.0),
                weight: system.weight,
            }
        })
    }

    /// 大きさを `ratio` 倍にした書体。
    pub fn scaled(&self, ratio: f32) -> Self {
        Self {
            size: self.size * ratio,
            ..self.clone()
        }
    }

    /// 一行に収める書式。折り返さず、はみ出す分は「…」にする。
    pub fn line(&self) -> Option<IDWriteTextFormat> {
        let format = self.format()?;
        // SAFETY: 作ったばかりの書式に設定するだけ。
        unsafe {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP).ok()?;
            format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
                .ok()?;
            let sign = DWRITE.with(|f| f.as_ref()?.CreateEllipsisTrimmingSign(&format).ok())?;
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                ..Default::default()
            };
            format.SetTrimming(&trimming, &sign).ok()?;
        }
        Some(format)
    }

    /// 一行に収め、真ん中に置く書式。
    pub fn centered(&self) -> Option<IDWriteTextFormat> {
        let format = self.line()?;
        // SAFETY: 作ったばかりの書式に設定するだけ。
        unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER) }.ok()?;
        Some(format)
    }

    /// 幅で折り返す書式。
    pub fn wrapping(&self) -> Option<IDWriteTextFormat> {
        self.format()
    }

    fn format(&self) -> Option<IDWriteTextFormat> {
        DWRITE.with(|factory| {
            // SAFETY: 名前と大きさを渡して書式を作るだけ。
            unsafe {
                factory
                    .as_ref()?
                    .CreateTextFormat(
                        &self.family,
                        None,
                        DWRITE_FONT_WEIGHT(self.weight),
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        self.size,
                        w!("ja-jp"),
                    )
                    .ok()
            }
        })
    }
}

/// `text` を `format` で、幅 `max_width` に収めたときの大きさ (DIP)。
pub fn measure(text: &str, format: &IDWriteTextFormat, max_width: f32) -> (f32, f32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    DWRITE.with(|factory| {
        let Some(factory) = factory.as_ref() else {
            return (0.0, 0.0);
        };
        // SAFETY: 文字列と書式を渡して測るだけ。
        unsafe {
            let Ok(layout) = factory.CreateTextLayout(&wide, format, max_width, f32::MAX) else {
                return (0.0, 0.0);
            };
            let mut metrics = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut metrics).is_err() {
                return (0.0, 0.0);
            }
            (metrics.widthIncludingTrailingWhitespace, metrics.height)
        }
    })
}

/// 矩形を DIP の Direct2D の矩形にする。
pub fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// `hwnd` の描く先を用意して `draw` に渡す。
///
/// 描く先は窓ではなく、**窓の GDI の画面 (HDC) に結びつける** (`BindDC`)。
/// 窓に直接描く描く先 (`CreateHwndRenderTarget`) は、裏で DXGI の画面を作る。
/// ストアアプリの入れ物の中ではそれが断られ (DXGI_ERROR_NOT_CURRENTLY_AVAILABLE)、
/// 窓は出るのに中身が白いままになった。CorvusSKK と Weasel も、GDI の画面に
/// 描く描く先を使っている。
///
/// 描く先はスレッドに一つで、描くたびに窓の画面へ結びつけ直す。描く命令は
/// これまでどおり Direct2D と DirectWrite である。**描く先が使えなくなったら
/// 捨てて、次に作り直す。**
pub fn with_target(hwnd: HWND, dpi: u32, draw: impl FnOnce(&ID2D1RenderTarget)) {
    let mut client = RECT::default();
    // SAFETY: 窓の大きさを尋ねるだけ。
    if let Err(e) = unsafe { GetClientRect(hwnd, &mut client) } {
        log::error(&format!("窓の大きさを尋ねられなかった: {}", e.message()));
        return;
    }
    let Some(target) = target() else {
        return;
    };
    // SAFETY: 自分の窓の画面を借りる。描き終えたら返す。
    let hdc = unsafe { GetDC(Some(hwnd)) };
    if hdc.is_invalid() {
        log::error("窓の画面を借りられなかった");
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let dpi = dpi as f32;
    // SAFETY: 描く手順どおり。借りた画面は、この中でだけ使う。
    let ended = unsafe {
        target.BindDC(hdc, &client).and_then(|()| {
            target.SetDpi(dpi, dpi);
            target.BeginDraw();
            draw(&target);
            target.EndDraw(None, None)
        })
    };
    // SAFETY: 借りた画面を返す。
    unsafe {
        ReleaseDC(Some(hwnd), hdc);
    }
    if let Err(e) = ended {
        log::error(&format!("描き終えられなかった: {}", e.message()));
        TARGET.with(|target| target.borrow_mut().take());
    }
}

/// このスレッドの描く先。無ければ作る。
fn target() -> Option<ID2D1DCRenderTarget> {
    if let Some(found) = TARGET.with(|target| target.borrow().clone()) {
        return Some(found);
    }
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        // CPU で描く。小窓には十分で、GPU の用意 (アプリごとにドライバを
        // 読み込む) を待たずに済む。Weasel でも、最初の打鍵が数百 ms 遅れる
        // 原因として挙がっている。
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        // 地を透かせるよう、アルファを持たせておく。
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        ..Default::default()
    };
    let created = D2D.with(|factory| {
        // SAFETY: 描く先を作るだけ。
        unsafe { factory.as_ref()?.CreateDCRenderTarget(&properties) }
            .inspect_err(|e| log::error(&format!("描く先を作れなかった: {}", e.message())))
            .ok()
    })?;
    TARGET.with(|target| *target.borrow_mut() = Some(created.clone()));
    Some(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_become_fractions() {
        let c = color(0xFF_80_00);
        assert!((c.r - 1.0).abs() < f32::EPSILON);
        assert!((c.g - 128.0 / 255.0).abs() < 1e-6);
        assert!(c.b.abs() < f32::EPSILON);
    }

    #[test]
    fn lengths_round_up_to_whole_pixels() {
        assert_eq!(pixels(10.0, 96), 10);
        assert_eq!(pixels(10.0, 144), 15);
        assert_eq!(pixels(10.1, 96), 11);
    }

    #[test]
    fn text_can_be_measured() {
        let format = Font::message().line().expect("書式を作れる");
        let (width, height) = measure("漢字", &format, f32::MAX);
        assert!(width > 0.0 && height > 0.0);
        let (wider, _) = measure("漢字漢字", &format, f32::MAX);
        assert!(wider > width);
    }
}
