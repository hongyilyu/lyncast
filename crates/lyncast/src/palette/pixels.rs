//! Images that lyncast decodes itself and hands GPUI as pixels. Nothing here goes through GPUI's
//! image loader, which reads a path with `fs::read` and so waits on a FIFO.

use std::io::Cursor;

use gpui::RenderImage;
use image::{Frame, ImageReader, Limits, RgbaImage};

/// Straight BGRA pixels, as GPUI draws them.
pub struct Bgra(pub RgbaImage);

impl Bgra {
    pub fn from_rgba(mut image: RgbaImage) -> Bgra {
        for pixel in image.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Bgra(image)
    }

    pub fn into_render_image(self) -> RenderImage {
        RenderImage::new([Frame::new(self.0)])
    }
}

/// The image in `bytes`, by its content, decoded within `limits` and scaled down to fit `fit`
/// with its shape kept. Fails with why it did not decode.
pub fn decode_fitted(bytes: &[u8], limits: Limits, fit: (u32, u32)) -> Result<Bgra, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .expect("reading a slice cannot fail");
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let fitted = if decoded.width() > fit.0 || decoded.height() > fit.1 {
        decoded.thumbnail(fit.0, fit.1)
    } else {
        decoded
    };
    Ok(Bgra::from_rgba(fitted.into_rgba8()))
}
