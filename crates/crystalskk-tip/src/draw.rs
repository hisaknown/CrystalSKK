//! Direct2D と DirectWrite で小窓を描くための下ごしらえ。
//!
//! GDI では小さな角の丸みがなめらかにならず、文字も粗い。**半透明も扱えない**
//! ので、窓の地を透かす (Acrylic) こともできない。そこで描くのは Direct2D に
//! 任せる。
//!
//! 座標はすべて DIP (100% のときの画素) で書き、拡大率は描く先に教える。
//! 窓の実際の大きさだけが画素である ([`pixels`])。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{HSTRING, w};

use crate::dpi;

thread_local! {
    /// Direct2D の工場。入力スレッドごとに一つ持つ (一つのスレッドでしか使わない)。
    static D2D: Option<ID2D1Factory> = {
        // SAFETY: 工場を作るだけ。
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }.ok()
    };
    /// DirectWrite の工場。
    static DWRITE: Option<IDWriteFactory> = {
        // SAFETY: 工場を作るだけ。
        unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ok()
    };
    /// 窓ごとの描く先。作るのは重いので、窓があるあいだ使い回す。
    static TARGETS: RefCell<Vec<(isize, ID2D1HwndRenderTarget)>> = const { RefCell::new(Vec::new()) };
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
    /// システムのメッセージの書体。読めなければ Yu Gothic UI の 9pt。
    pub fn message() -> Self {
        dpi::message_font_spec().map_or_else(
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
        )
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
/// 描く先は窓ごとに使い回す。大きさと拡大率は、描くたびに合わせる。**描く先が
/// 使えなくなったら (画面の設定が変わったときなど) 捨てて、次に作り直す。**
pub fn with_target(hwnd: HWND, dpi: u32, draw: impl FnOnce(&ID2D1HwndRenderTarget)) {
    let mut client = RECT::default();
    // SAFETY: 窓の大きさを尋ねるだけ。
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let size = D2D_SIZE_U {
        width: u32::try_from(client.right - client.left).unwrap_or(0),
        height: u32::try_from(client.bottom - client.top).unwrap_or(0),
    };
    let Some(target) = target(hwnd, size) else {
        return;
    };
    #[allow(clippy::cast_precision_loss)]
    let dpi = dpi as f32;
    // SAFETY: 描く手順どおり。
    let ended = unsafe {
        let _ = target.Resize(&size);
        target.SetDpi(dpi, dpi);
        target.BeginDraw();
        draw(&target);
        target.EndDraw(None, None)
    };
    if ended.is_err() {
        forget(hwnd);
    }
}

/// 窓の描く先を捨てる。窓を壊すときに呼ぶ。
pub fn forget(hwnd: HWND) {
    TARGETS.with(|targets| targets.borrow_mut().retain(|(h, _)| *h != hwnd.0 as isize));
}

fn target(hwnd: HWND, size: D2D_SIZE_U) -> Option<ID2D1HwndRenderTarget> {
    let key = hwnd.0 as isize;
    if let Some(found) = TARGETS.with(|targets| {
        targets
            .borrow()
            .iter()
            .find(|(h, _)| *h == key)
            .map(|(_, t)| t.clone())
    }) {
        return Some(found);
    }
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        // 地を透かせるよう、アルファを持たせておく。
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        ..Default::default()
    };
    let window = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: size,
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    let created = D2D.with(|factory| {
        // SAFETY: 自分の窓に描く先を作るだけ。
        unsafe {
            factory
                .as_ref()?
                .CreateHwndRenderTarget(&properties, &window)
                .ok()
        }
    })?;
    TARGETS.with(|targets| targets.borrow_mut().push((key, created.clone())));
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
