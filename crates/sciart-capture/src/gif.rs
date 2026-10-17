//! A looping 256-colour GIF encoder (CLI-R8, ADR-0025).
//!
//! Frames are consumed one at a time: each gets its own local colour table, built from its
//! own pixels (exactly, when it uses at most 256 colours; otherwise through a fixed
//! 6 × 6 × 6 cube plus a 40-step grey ramp). A recording therefore never holds more than one
//! frame in memory, and every frame stays deterministic.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use gif::{Encoder, Frame, Repeat};

use crate::Image;

/// 216 cube levels + 40 greys = the 256 entries a GIF allows.
const CUBE: [u8; 6] = [0, 51, 102, 153, 204, 255];
const GREYS: usize = 40;

/// Write `frames` to `path` as one infinitely looping GIF at `fps`, one frame at a time.
///
/// An empty iterator, a zero `fps` or a size change between frames is an error.
pub fn write_gif<I>(frames: I, path: &Path, fps: u16) -> std::io::Result<()>
where
    I: IntoIterator<Item = Image>,
{
    if fps == 0 {
        return Err(invalid("fps must be at least 1"));
    }
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(invalid("no frames to encode"));
    };
    let width =
        u16::try_from(first.width()).map_err(|_| invalid("frame width does not fit a GIF"))?;
    let height =
        u16::try_from(first.height()).map_err(|_| invalid("frame height does not fit a GIF"))?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    // A minimal global table; every frame carries its own local table.
    let mut encoder = Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        width,
        height,
        &[0, 0, 0, 0, 0, 0],
    )
    .map_err(std::io::Error::other)?;
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(std::io::Error::other)?;
    // GIF delays are in hundredths of a second.
    let delay = ((100.0 / f64::from(fps)).round() as u16).max(1);
    let fixed = fixed_palette();
    let mut buffer = Vec::with_capacity(first.width() * first.height());
    let mut palette = Vec::with_capacity(768);
    let mut index_of = HashMap::new();
    let mut colours = Vec::new();
    for (number, image) in std::iter::once(first).chain(frames).enumerate() {
        if (image.width(), image.height()) != (usize::from(width), usize::from(height)) {
            return Err(invalid("every GIF frame must have the same size"));
        }
        build_palette(&image, &fixed, &mut palette, &mut index_of, &mut colours);
        // Reuse one index buffer for every frame instead of allocating 36 000 of them.
        buffer.clear();
        for px in image.rgb().chunks_exact(3) {
            let index = *index_of
                .get(&key(px))
                .ok_or_else(|| invalid("frame colour missing from its palette"))?;
            buffer.push(index);
        }
        let frame = Frame {
            width,
            height,
            delay,
            palette: Some(palette.clone()),
            buffer: Cow::Borrowed(&buffer),
            ..Frame::default()
        };
        encoder
            .write_frame(&frame)
            .map_err(|e| invalid(&format!("frame {number}: {e}")))?;
    }
    encoder.into_inner()?.flush()
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

fn key(px: &[u8]) -> u32 {
    (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2])
}

/// Fill `palette` and `index_of` for one frame; exact when it has at most 256 colours,
/// the fixed cube otherwise. The scratch collections are reused across frames.
fn build_palette(
    image: &Image,
    fixed: &[[u8; 3]],
    palette: &mut Vec<u8>,
    index_of: &mut HashMap<u32, u8>,
    colours: &mut Vec<u32>,
) {
    colours.clear();
    colours.extend(image.rgb().chunks_exact(3).map(key));
    colours.sort_unstable();
    colours.dedup();
    palette.clear();
    index_of.clear();
    if colours.len() <= 256 {
        for (index, &colour) in colours.iter().enumerate() {
            let index = u8::try_from(index).unwrap_or(0);
            index_of.insert(colour, index);
            palette.extend([(colour >> 16) as u8, (colour >> 8) as u8, colour as u8]);
        }
        // A GIF colour table has at least two entries; the encoder pads the rest.
        if colours.len() == 1 {
            palette.extend([0, 0, 0]);
        }
    } else {
        for colour in fixed {
            palette.extend(colour);
        }
        for &colour in colours.iter() {
            let px = [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8];
            index_of.insert(colour, nearest(fixed, px));
        }
    }
}

/// 6 × 6 × 6 RGB cube (216) followed by 40 evenly spaced greys.
fn fixed_palette() -> Vec<[u8; 3]> {
    let mut colours = Vec::with_capacity(216 + GREYS);
    for r in CUBE {
        for g in CUBE {
            for b in CUBE {
                colours.push([r, g, b]);
            }
        }
    }
    for step in 0..GREYS {
        // 0..=255 across the ramp; `GREYS - 1` is non-zero.
        let level = (step * 255 / (GREYS - 1)) as u8;
        colours.push([level, level, level]);
    }
    colours
}

fn nearest(palette: &[[u8; 3]], px: [u8; 3]) -> u8 {
    let mut best = 0usize;
    let mut best_distance = u32::MAX;
    for (index, &entry) in palette.iter().enumerate() {
        let distance = (0..3)
            .map(|channel| {
                let a = i32::from(px[channel]);
                let b = i32::from(entry[channel]);
                (a - b) * (a - b)
            })
            .sum::<i32>() as u32;
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best as u8
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use sciart_render::Rgb;

    use super::*;

    fn image(width: usize, height: usize, fill: Rgb) -> Image {
        Image::new(width, height, fill)
    }

    fn decode(bytes: Vec<u8>) -> (u16, u16, Repeat, Vec<Vec<u8>>) {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(Cursor::new(bytes)).unwrap();
        let (width, height) = (decoder.width(), decoder.height());
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push(frame.buffer.to_vec());
        }
        (width, height, decoder.repeat(), frames)
    }

    #[test]
    fn cli_r8_gif_is_a_looping_animation_with_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.gif");
        let frames = [
            image(4, 3, Rgb(12, 12, 18)),
            image(4, 3, Rgb(255, 0, 0)),
            image(4, 3, Rgb(0, 0, 255)),
        ];
        write_gif(frames, &path, 30).unwrap();
        let (width, height, repeat, decoded) = decode(std::fs::read(path).unwrap());
        assert_eq!((width, height), (4, 3));
        assert_eq!(repeat, Repeat::Infinite, "the animation loops");
        assert_eq!(decoded.len(), 3);
        // Every pixel is one fill, so each frame's exact palette preserves it.
        assert!(decoded[1].chunks_exact(4).all(|px| px == [255, 0, 0, 255]));
        assert!(decoded[2].chunks_exact(4).all(|px| px == [0, 0, 255, 255]));
    }

    #[test]
    fn cli_r8_gif_quantizes_frames_over_256_colours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.gif");
        let (width, height) = (32, 16);
        let mut img = image(width, height, Rgb(0, 0, 0));
        for y in 0..height {
            for x in 0..width {
                let rgb = Rgb((x * 8) as u8, (y * 16) as u8, ((x + y) * 4) as u8);
                img.rect(x, y, 1, 1, rgb);
            }
        }
        write_gif([img], &path, 60).unwrap();
        let (w, h, _, decoded) = decode(std::fs::read(path).unwrap());
        assert_eq!((w, h), (32, 16));
        assert_eq!(decoded.len(), 1);
        assert_eq!(
            decoded[0].chunks_exact(4).count(),
            usize::from(w) * usize::from(h)
        );
    }

    #[test]
    fn cli_r8_gif_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.gif");
        assert!(write_gif(std::iter::empty(), &path, 30).is_err());
        assert!(write_gif([image(2, 2, Rgb(0, 0, 0))], &path, 0).is_err());
    }
}
