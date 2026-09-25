//! 絵を、Windows が出せる形にする。**ビルドのときにだけ使う。**
//!
//! 絵の正本は `assets/icons/` の SVG である。ここで各大きさに描き、
//!
//! - 入力モードの絵は、**濃さ** (透過度) だけを取り出す。色は動くときに
//!   タスクバーの明るさに合わせて付ける。SVG の色は使わない。
//! - 顔の絵は、色付きのまま `.ico` にする。
//!
//! 描くのは `resvg` で、純 Rust である (C のツールチェインを求めない)。
//! 文字を描く機能は外してある。**SVG の文字はアウトライン化しておく。**
//! 文字のままだと、描いた結果がビルドする機械の字体に左右される。

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

/// 描けなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// 描いた絵。上の行が先で、一画素 RGBA の 4 バイト。**透過度は掛け合わせ
/// ていない** (色の値は透過度に左右されない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub size: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// 濃さ (透過度) だけ。一画素 1 バイト。
    pub fn coverage(&self) -> Vec<u8> {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect()
    }
}

/// SVG を `size` 画素四方に描く。絵は四角いっぱいに引き伸ばす。
pub fn rasterize(svg: &str, size: u32) -> Result<Image, Error> {
    if svg.contains("<text") {
        return Err(Error(
            "SVG に文字 (<text>) が残っています。アウトライン化してください".to_owned(),
        ));
    }
    let tree = Tree::from_str(svg, &Options::default())
        .map_err(|e| Error(format!("SVG を読めません: {e}")))?;
    let mut pixmap =
        Pixmap::new(size, size).ok_or_else(|| Error(format!("{size} 画素の絵は作れません")))?;
    let drawn = tree.size();
    let transform =
        Transform::from_scale(size as f32 / drawn.width(), size as f32 / drawn.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia は透過度を掛け合わせた値で持つ。戻しておく。
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Ok(Image { size, rgba })
}

/// 色付きの絵を `.ico` にするときの大きさ。一覧の小さな絵から、大きく出す
/// 場面までを覆う。
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// `.ico` にする。大きさの違う絵を何枚でも入れられる。
///
/// 256 画素の絵は PNG で、それより小さい絵は 32 ビットの BMP で入れる。
/// どちらも Windows Vista からの形で、透過度がそのまま効く。
pub fn ico(images: &[Image]) -> Result<Vec<u8>, Error> {
    let encoded: Vec<(u32, Vec<u8>)> = images
        .iter()
        .map(|image| {
            let body = if image.size >= 256 {
                png(image)?
            } else {
                bmp(image)
            };
            Ok((image.size, body))
        })
        .collect::<Result<_, Error>>()?;

    let count = u16::try_from(encoded.len()).map_err(|_| Error("絵が多すぎます".to_owned()))?;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // 予約
    out.extend_from_slice(&1u16.to_le_bytes()); // 種別: アイコン
    out.extend_from_slice(&count.to_le_bytes());

    let mut offset = 6 + 16 * encoded.len();
    for (size, body) in &encoded {
        // 256 画素は 0 で表す決まり。
        let dimension = u8::try_from(*size).unwrap_or(0);
        out.push(dimension);
        out.push(dimension);
        out.push(0); // 色数: 32 ビットなので 0
        out.push(0); // 予約
        out.extend_from_slice(&1u16.to_le_bytes()); // 面数
        out.extend_from_slice(&32u16.to_le_bytes()); // ビット数
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += body.len();
    }
    for (_, body) in &encoded {
        out.extend_from_slice(body);
    }
    Ok(out)
}

/// PNG にする。
pub fn png(image: &Image) -> Result<Vec<u8>, Error> {
    let mut pixmap = Pixmap::new(image.size, image.size)
        .ok_or_else(|| Error(format!("{} 画素の絵は作れません", image.size)))?;
    for (target, pixel) in pixmap
        .pixels_mut()
        .iter_mut()
        .zip(image.rgba.as_chunks::<4>().0)
    {
        *target = resvg::tiny_skia::ColorU8::from_rgba(pixel[0], pixel[1], pixel[2], pixel[3])
            .premultiply();
    }
    pixmap
        .encode_png()
        .map_err(|e| Error(format!("PNG にできません: {e}")))
}

/// `.ico` の中の BMP にする。下の行が先で、高さは覆いの分を足して二倍に書く。
fn bmp(image: &Image) -> Vec<u8> {
    let width = image.size as usize;
    // 覆いは 4 バイト境界に揃える。32 ビットの絵では使わないので全て 0。
    let mask_row = width.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + width * width * 4 + mask_row * width);

    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(image.size as i32).to_le_bytes());
    out.extend_from_slice(&((image.size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 圧縮なし
    out.extend_from_slice(&0u32.to_le_bytes()); // 大きさは省略してよい
    out.extend_from_slice(&[0u8; 16]); // 解像度と色表の欄

    for row in (0..width).rev() {
        for column in 0..width {
            let at = (row * width + column) * 4;
            let [r, g, b, a] = [
                image.rgba[at],
                image.rgba[at + 1],
                image.rgba[at + 2],
                image.rgba[at + 3],
            ];
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out.extend_from_slice(&vec![0u8; mask_row * width]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4">
        <rect x="1" y="1" width="2" height="2" fill="#123456"/></svg>"##;

    #[test]
    fn the_drawing_fills_the_square_it_is_given() {
        let image = rasterize(SQUARE, 16).unwrap();
        let coverage = image.coverage();
        assert_eq!(coverage.len(), 16 * 16);
        assert_eq!(coverage[0], 0, "角は透明");
        assert_eq!(coverage[8 * 16 + 8], 255, "真ん中は塗られている");
        // 色は掛け合わせずに持つ。
        assert_eq!(&image.rgba[(8 * 16 + 8) * 4..][..3], &[0x12, 0x34, 0x56]);
    }

    #[test]
    fn text_left_in_the_drawing_is_refused() {
        let svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><text>あ</text></svg>"#;
        let error = rasterize(svg, 16).unwrap_err();
        assert!(error.to_string().contains("アウトライン"));
    }

    #[test]
    fn an_ico_lists_every_size() {
        let images: Vec<Image> = [16, 32, 256]
            .iter()
            .map(|size| rasterize(SQUARE, *size).unwrap())
            .collect();
        let bytes = ico(&images).unwrap();
        assert_eq!(&bytes[..6], &[0, 0, 1, 0, 3, 0], "アイコンが三枚");
        assert_eq!(bytes[6], 16);
        assert_eq!(bytes[6 + 16], 32);
        assert_eq!(bytes[6 + 32], 0, "256 は 0 と書く");
        // 256 の絵は PNG で入っている。
        let offset = u32::from_le_bytes(bytes[6 + 32 + 12..][..4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[offset..offset + 4], b"\x89PNG");
    }

    #[test]
    fn a_bmp_entry_is_written_bottom_up_with_room_for_the_mask() {
        // 一行目 (上) だけ印を付けて、最後の行に来ることを見る。
        let mut rgba = vec![0u8; 2 * 2 * 4];
        rgba[..4].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let bytes = bmp(&Image { size: 2, rgba });

        let height = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(height, 4, "絵と覆いの分で二倍");
        // 下の行から書くので、上の行は後ろに来る。並びは BGRA。
        let top_row = 40 + 2 * 4;
        assert_eq!(&bytes[top_row..top_row + 4], &[0x33, 0x22, 0x11, 0x44]);
    }
}
