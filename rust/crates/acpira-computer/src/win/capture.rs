//! A picture of one window. The window is asked to draw itself into a bitmap, so one that is covered or behind
//! others comes out whole; a minimized one has nothing to draw.

use super::apps::{bounds_of, handle};
use crate::picture::{QUALITY, Shot, fit};
use crate::tree::Rect;
use std::ffi::c_void;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC, SelectObject};
use windows::Win32::Graphics::Imaging::{
  CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGR, IWICBitmapFrameEncode, IWICBitmapSource, IWICFormatConverter, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
  WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{Interface, PWSTR};

/// `PW_RENDERFULLCONTENT`: include what the window draws with the graphics card (browsers, most newer apps).
const FULL_CONTENT: u32 = 0x0000_0002;

/// The window's pixels, top row first, four bytes each (blue, green, red, unused), and their size.
fn pixels(hwnd: isize, size: (i32, i32)) -> Result<Vec<u8>, String> {
  let info = BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: size.0, biHeight: -size.1, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
    ..Default::default()
  };
  // SAFETY: every object made here is released here; `bits` points at the bitmap's own memory, which is copied
  // before the bitmap goes.
  unsafe {
    let screen = GetDC(None);
    let memory = CreateCompatibleDC(Some(screen));
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap = CreateDIBSection(Some(screen), &raw const info, DIB_RGB_COLORS, &raw mut bits, None, 0);
    let result = match bitmap {
      Ok(bitmap) if !bits.is_null() => {
        let previous = SelectObject(memory, bitmap.into());
        let drawn = PrintWindow(handle(hwnd), memory, PRINT_WINDOW_FLAGS(FULL_CONTENT)).as_bool();
        let _ = GdiFlush();
        let copied = std::slice::from_raw_parts(bits.cast::<u8>(), size.0 as usize * size.1 as usize * 4).to_vec();
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        if drawn { Ok(copied) } else { Err("the window did not draw itself for a picture".to_string()) }
      }
      _ => Err("no memory for a picture of the window".to_string()),
    };
    let _ = DeleteDC(memory);
    ReleaseDC(None, screen);
    result
  }
}

/// `source` scaled to `size` and written as a JPEG.
fn encode(source: &[u8], from: (u32, u32), size: (u32, u32)) -> windows::core::Result<Vec<u8>> {
  // SAFETY: plain COM calls on objects that live until the end of the block; the buffers passed outlive the calls
  // that read or fill them.
  unsafe {
    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let bitmap = factory.CreateBitmapFromMemory(from.0, from.1, &GUID_WICPixelFormat32bppBGR, from.0 * 4, source)?;
    let mut image: IWICBitmapSource = bitmap.cast()?;
    if size != from {
      let scaler = factory.CreateBitmapScaler()?;
      scaler.Initialize(&image, size.0, size.1, WICBitmapInterpolationModeFant)?;
      image = scaler.cast()?;
    }
    // JPEG holds three bytes a pixel.
    let converter: IWICFormatConverter = factory.CreateFormatConverter()?;
    converter.Initialize(&image, &GUID_WICPixelFormat24bppBGR, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;

    let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
    let encoder = factory.CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame: Option<IWICBitmapFrameEncode> = None;
    let mut options: Option<IPropertyBag2> = None;
    encoder.CreateNewFrame(&raw mut frame, &raw mut options)?;
    let (Some(frame), Some(options)) = (frame, options) else { return Err(windows::core::Error::empty()) };
    let mut name: Vec<u16> = "ImageQuality".encode_utf16().chain(std::iter::once(0)).collect();
    let option = PROPBAG2 { pstrName: PWSTR(name.as_mut_ptr()), ..Default::default() };
    let quality = VARIANT::from(QUALITY as f32);
    options.Write(1, &raw const option, &raw const quality)?;
    frame.Initialize(&options)?;
    frame.SetSize(size.0, size.1)?;
    let mut format = GUID_WICPixelFormat24bppBGR;
    frame.SetPixelFormat(&raw mut format)?;
    frame.WriteSource(&converter, std::ptr::null())?;
    frame.Commit()?;
    encoder.Commit()?;

    let mut stat = STATSTG::default();
    stream.Stat(&raw mut stat, STATFLAG_NONAME)?;
    stream.Seek(0, STREAM_SEEK_SET, None)?;
    let mut jpeg = vec![0u8; stat.cbSize as usize];
    let mut read = 0u32;
    stream.Read(jpeg.as_mut_ptr().cast(), jpeg.len() as u32, Some(&raw mut read)).ok()?;
    jpeg.truncate(read as usize);
    Ok(jpeg)
  }
}

/// A picture of the window `hwnd`, which the user sees as `frame`.
pub fn window(hwnd: isize, frame: Rect) -> Result<Shot, String> {
  // The window draws its whole rectangle, which is wider than what is seen of it by a border nobody sees.
  let whole = bounds_of(hwnd);
  let size = (whole.w as i32, whole.h as i32);
  if size.0 <= 0 || size.1 <= 0 || frame.w < 1.0 || frame.h < 1.0 {
    return Err("the window has no surface to picture".into());
  }
  let drawn = pixels(hwnd, size)?;
  // Cut to what is seen, so a point in the picture is a point in the window as the outline places it.
  let left = ((frame.x - whole.x).max(0.0) as usize).min(size.0 as usize - 1);
  let top = ((frame.y - whole.y).max(0.0) as usize).min(size.1 as usize - 1);
  let width = (frame.w as usize).min(size.0 as usize - left);
  let height = (frame.h as usize).min(size.1 as usize - top);
  let mut seen = Vec::with_capacity(width * height * 4);
  for row in top..top + height {
    let start = (row * size.0 as usize + left) * 4;
    seen.extend_from_slice(drawn.get(start..start + width * 4).unwrap_or_default());
  }
  if seen.len() != width * height * 4 {
    return Err("the window changed size while it was pictured".into());
  }
  let (out_width, out_height, scale) = fit((width as f64, height as f64), (width, height));
  let jpeg = encode(&seen, (width as u32, height as u32), (out_width, out_height)).map_err(|error| format!("the picture could not be encoded ({error})"))?;
  Ok(Shot { jpeg, width: out_width, height: out_height, scale })
}
