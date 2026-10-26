//! Animated GIFs the way a model can see them. A model reads one still per image block: Claude takes a GIF's first
//! frame, OpenAI only non-animated GIFs, and Gemini does not list GIF among its image types (2026-10-25), so a pasted
//! screen recording reached the model as its opening frame and nothing else. `sheet` samples the frames evenly in time
//! and lays them out as one PNG grid, small enough that a provider does not scale it down again; `FrameSheet::note` is
//! the text that tells the model what the grid is. Both the engine's prompt staging and the built-in agent's `read`
//! use it

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::imageops::{self, FilterType};
use image::{AnimationDecoder, DynamicImage, ImageFormat, Rgb, RgbImage};

/// The sheet's longer side: Claude scales a larger image down to this, so anything beyond it is spent for nothing
const MAX_SIDE: u32 = 1568;
/// The sheet's area: Claude also scales an image over about 1.15 megapixels down
const MAX_AREA: f64 = 1_100_000.0;
/// Pixels between two cells, in `GAP_COLOR`: mid grey parts both dark and light frames
const GAP: u32 = 6;
const GAP_COLOR: Rgb<u8> = Rgb([128, 128, 128]);
/// Cells on a sheet: a short loop still gets enough to show its motion, a long one no more than stay legible
const MIN_CELLS: usize = 6;
const MAX_CELLS: usize = 16;
/// Frames counted before giving up on a file (the timing pass is cheap, but not free)
const MAX_FRAMES: usize = 10_000;
/// Canvas pixels decoded to reach the last sampled frame; a larger file goes to the model as it is
const MAX_DECODED_PIXELS: u64 = 1_000_000_000;

/// An animated GIF as one picture of its frames
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSheet {
  /// The grid, a PNG
  pub png: Vec<u8>,
  /// The GIF's own frame count, duration of one loop and canvas size
  pub frames: usize,
  pub duration_ms: u64,
  pub width: u32,
  pub height: u32,
  /// Where each cell was taken, in cell order
  pub at_ms: Vec<u64>,
  pub cols: u32,
  pub rows: u32,
}

impl FrameSheet {
  /// What the model is told about the sheet: what the GIF was, how the cells are laid out and when each was taken.
  /// `original`: where the GIF itself can be read, for a model that wants more frames than the sheet holds
  pub fn note(&self, label: &str, original: Option<&str>) -> String {
    let times = self.at_ms.iter().map(|t| format!("{:.1} s", *t as f64 / 1000.0)).collect::<Vec<_>>().join(", ");
    let mut out = format!(
      "Animated GIF {label}: {} frames over {:.1} s, {}x{} pixels. Models see only a GIF's first frame, so the attached image is a \
       sheet of {} frames sampled evenly in time, {} columns by {} rows, read left to right and top to bottom, taken at {times}.",
      self.frames,
      self.duration_ms as f64 / 1000.0,
      self.width,
      self.height,
      self.at_ms.len(),
      self.cols,
      self.rows,
    );
    if let Some(path) = original {
      out.push_str(&format!(" The original GIF is at {path}; extract more frames from it with a command if you need them."));
    }
    out
  }
}

/// The sheet of an animated GIF; None for a still GIF, one that cannot be read, or one too large to decode here,
/// all of which go to the model as they are
pub fn sheet(bytes: &[u8]) -> Option<FrameSheet> {
  let (width, height, shown) = timing(bytes)?;
  if shown.len() < 2 || width == 0 || height == 0 {
    return None;
  }
  let starts: Vec<u64> = shown.iter().scan(0u64, |t, d| Some(std::mem::replace(t, *t + u64::from(*d)))).collect();
  let duration_ms: u64 = shown.iter().map(|d| u64::from(*d)).sum();
  let want = (duration_ms as f64 / 1000.0 * 1.2).ceil() as usize;
  let picked = pick(&starts, want.clamp(MIN_CELLS, MAX_CELLS).min(shown.len()));
  let last = *picked.last()? as u64;
  if (last + 1) * u64::from(width) * u64::from(height) > MAX_DECODED_PIXELS {
    return None;
  }
  let (cols, rows, cw, ch) = layout(picked.len() as u32, width, height);
  // Decode up to the last sampled frame, keeping only the sampled ones (scaled at once: a whole loop of full frames
  // can run to hundreds of MB)
  let mut cells = Vec::with_capacity(picked.len());
  let mut next = picked.iter().copied().peekable();
  for (i, frame) in GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames().enumerate() {
    let Some(&k) = next.peek() else { break };
    // A cut-off file shows what it has, as a browser does
    let Ok(frame) = frame else { break };
    if i == k {
      cells.push(imageops::resize(frame.buffer(), cw, ch, FilterType::Triangle));
      next.next();
    }
  }
  if cells.len() < 2 {
    return None;
  }
  let at_ms: Vec<u64> = picked[..cells.len()].iter().map(|&k| starts[k]).collect();
  let mut grid = RgbImage::from_pixel(cols * cw + (cols - 1) * GAP, rows * ch + (rows - 1) * GAP, GAP_COLOR);
  for (n, cell) in cells.iter().enumerate() {
    let (x0, y0) = ((n as u32 % cols) * (cw + GAP), (n as u32 / cols) * (ch + GAP));
    for (x, y, p) in cell.enumerate_pixels() {
      // Transparent pixels over white, as on a page
      let a = u32::from(p[3]);
      let over = |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
      grid.put_pixel(x0 + x, y0 + y, Rgb([over(p[0]), over(p[1]), over(p[2])]));
    }
  }
  let mut png = vec![];
  DynamicImage::ImageRgb8(grid).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).ok()?;
  Some(FrameSheet { png, frames: shown.len(), duration_ms, width, height, at_ms, cols, rows })
}

/// The canvas size and how long each frame shows, in ms, without decoding any pixels
fn timing(bytes: &[u8]) -> Option<(u32, u32, Vec<u32>)> {
  let mut options = gif::DecodeOptions::new();
  options.skip_frame_decoding(true);
  let mut decoder = options.read_info(Cursor::new(bytes)).ok()?;
  let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
  let mut shown = vec![];
  // A cut-off file counts the frames before the cut
  while let Ok(Some(frame)) = decoder.read_next_frame() {
    // Browsers show a frame of 0 or 10 ms for 100 ms, and so does every GIF player people record for
    shown.push(if frame.delay <= 1 { 100 } else { u32::from(frame.delay) * 10 });
    if shown.len() > MAX_FRAMES {
      return None;
    }
  }
  Some((width, height, shown))
}

/// `n` frames evenly spread over the loop, from the first frame to the last: for each moment the frame on screen
/// then. Frames held longer than a step are picked once
fn pick(starts: &[u64], n: usize) -> Vec<usize> {
  let last = *starts.last().unwrap_or(&0);
  let mut out: Vec<usize> = vec![];
  for i in 0..n.max(2) as u64 {
    let t = last * i / (n.max(2) as u64 - 1);
    let k = starts.partition_point(|s| *s <= t).saturating_sub(1);
    if out.last() != Some(&k) {
      out.push(k);
    }
  }
  out
}

/// Columns, rows and the cell size for `n` frames of `w` x `h`: the largest cells within `MAX_SIDE` and `MAX_AREA`,
/// never enlarged; among equal sizes the grid with fewer empty cells, then the squarer one
fn layout(n: u32, w: u32, h: u32) -> (u32, u32, u32, u32) {
  let scale = |cols: u32, rows: u32| {
    let across = f64::from(MAX_SIDE.saturating_sub((cols - 1) * GAP)) / f64::from(cols * w);
    let down = f64::from(MAX_SIDE.saturating_sub((rows - 1) * GAP)) / f64::from(rows * h);
    let area = (MAX_AREA / (f64::from(cols * w) * f64::from(rows * h))).sqrt();
    across.min(down).min(area).min(1.0)
  };
  let rank = |cols: u32| {
    let rows = n.div_ceil(cols);
    let squareness = (f64::from(cols * w) / f64::from(rows * h)).ln().abs();
    ((scale(cols, rows) * 1000.0).round() as i64, -i64::from(cols * rows - n), -(squareness * 1000.0) as i64)
  };
  let cols = (1..=n).max_by_key(|c| rank(*c)).unwrap_or(1);
  let rows = n.div_ceil(cols);
  let s = scale(cols, rows);
  (cols, rows, ((f64::from(w) * s) as u32).max(1), ((f64::from(h) * s) as u32).max(1))
}

/// A GIF of solid frames, one colour each, shown `delay_cs` hundredths of a second apiece: test material for the
/// engine's suites as well as this module's
#[cfg(any(test, feature = "mock"))]
#[expect(clippy::unwrap_used, reason = "test material: encoding a few solid frames into a Vec cannot fail")]
pub fn solid_gif(w: u16, h: u16, colours: &[[u8; 3]], delay_cs: u16) -> Vec<u8> {
  let mut out = vec![];
  {
    let mut encoder = gif::Encoder::new(&mut out, w, h, &[]).unwrap();
    encoder.set_repeat(gif::Repeat::Infinite).unwrap();
    for c in colours {
      let frame = gif::Frame {
        width: w,
        height: h,
        delay: delay_cs,
        palette: Some(vec![c[0], c[1], c[2], 0, 0, 0]),
        buffer: std::borrow::Cow::Owned(vec![0; usize::from(w) * usize::from(h)]),
        ..gif::Frame::default()
      };
      encoder.write_frame(&frame).unwrap();
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  const RED: [u8; 3] = [255, 0, 0];
  const GREEN: [u8; 3] = [0, 255, 0];
  const BLUE: [u8; 3] = [0, 0, 255];

  #[test]
  fn an_animation_becomes_a_grid_of_its_frames_in_order() {
    let gif = solid_gif(40, 20, &[RED, GREEN, BLUE], 50);
    let s = sheet(&gif).expect("three frames make a sheet");
    assert_eq!((s.frames, s.duration_ms, s.width, s.height, s.at_ms.clone()), (3, 1500, 40, 20, vec![0, 500, 1000]));
    // Small frames are never enlarged: wide cells stack into the squarer sheet, one column, read back from the PNG
    let png = image::load_from_memory_with_format(&s.png, ImageFormat::Png).unwrap().to_rgb8();
    assert_eq!((s.cols, s.rows), (1, 3));
    assert_eq!(png.dimensions(), (40, 3 * 20 + 2 * GAP));
    assert_eq!([png.get_pixel(10, 10).0, png.get_pixel(10, 20 + GAP + 10).0, png.get_pixel(10, 2 * (20 + GAP) + 10).0], [RED, GREEN, BLUE]);
    assert_eq!(png.get_pixel(10, 20).0, GAP_COLOR.0);
    let note = s.note("clip.gif", Some("/tmp/clip.gif"));
    assert!(note.starts_with("Animated GIF clip.gif: 3 frames over 1.5 s, 40x20 pixels."), "{note}");
    assert!(note.contains("1 columns by 3 rows") && note.contains("taken at 0.0 s, 0.5 s, 1.0 s.") && note.ends_with("if you need them."), "{note}");
  }

  #[test]
  fn a_still_or_broken_gif_has_no_sheet() {
    assert_eq!(sheet(&solid_gif(8, 8, &[RED], 10)), None);
    assert_eq!(sheet(b"GIF89a not really"), None);
    assert_eq!(sheet(b"\x89PNG\r\n\x1a\n"), None);
  }

  #[test]
  fn a_long_animation_is_sampled_evenly_in_time_up_to_the_last_frame() {
    // 40 frames of 1 s: 16 cells from 0 s to 39 s
    let colours: Vec<[u8; 3]> = (0..40u8).map(|i| [i * 6, 0, 0]).collect();
    let s = sheet(&solid_gif(64, 32, &colours, 100)).unwrap();
    assert_eq!(s.at_ms.len(), MAX_CELLS);
    assert_eq!((s.at_ms[0], *s.at_ms.last().unwrap()), (0, 39_000));
    // A wide frame: fewer columns than a square grid would take, and the sheet stays within the size providers keep
    let png = image::load_from_memory(&s.png).unwrap();
    assert!(png.width() <= MAX_SIDE && png.height() <= MAX_SIDE && f64::from(png.width() * png.height()) <= MAX_AREA * 1.05);
  }

  #[test]
  fn zero_delays_count_as_browsers_show_them_and_held_frames_are_picked_once() {
    let (_, _, shown) = timing(&solid_gif(4, 4, &[RED, GREEN], 0)).unwrap();
    assert_eq!(shown, [100, 100]);
    // Moments every 500 ms over 3 s: a frame on screen across two of them is one cell
    assert_eq!(pick(&[0, 1000, 2000, 3000], 7), [0, 1, 2, 3]);
    // A brief frame between two moments is not sampled; the last frame always is
    assert_eq!(pick(&[0, 100, 5000, 5100], 8), [0, 1, 3]);
  }

  #[test]
  fn the_grid_follows_the_frame_shape() {
    // 16 cells of 640x316 (the screen recording that prompted this): no empty cell, each as large as the area allows
    let (cols, rows, cw, ch) = layout(16, 640, 316);
    assert_eq!(cols * rows, 16);
    assert!(cols * cw + (cols - 1) * GAP <= MAX_SIDE && rows * ch + (rows - 1) * GAP <= MAX_SIDE);
    assert!(cw >= 370 && ch >= 180, "{cw}x{ch}");
    // Very wide frames stack in fewer columns
    assert!(layout(12, 1600, 200).0 < layout(12, 400, 400).0);
  }
}
