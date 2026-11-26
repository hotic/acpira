//! What the user sees of the agent at work: a frame around the window it works in, its own pointer, and the clones
//! that show a batch's steps (`crate::motion`).
//!
//! Layered windows that take no input and never become active, kept directly above the target window in the stacking
//! order and not above everything: what covers the target covers the frame and the pointers too. Their pixels come
//! from `crate::paint`. They live on the process's main thread, which runs the message loop; commands arrive from the
//! thread that serves requests.

use super::apps::{frame_of, handle, minimized};
use crate::motion::{self, Arc as Move};
use crate::paint::{self, Bitmap};
use crate::tree::Rect;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
  CreateWindowExW, DefWindowProcW, DispatchMessageW, GW_HWNDPREV, GWL_EXSTYLE, GetMessageW, GetWindow, GetWindowLongPtrW, HTTRANSPARENT, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, IsWindowVisible, IsZoomed, KillTimer, MSG, PostMessageW, RegisterClassW,
  SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetTimer, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_NCHITTEST, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
  WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

const RIPPLE: Duration = Duration::from_millis(380);
/// Without a command for this long the overlay fades.
const IDLE: Duration = Duration::from_secs(6);
const FADE: Duration = Duration::from_millis(300);
const TICK: u32 = 16;
/// Corner radius of a window on Windows 11, in points; earlier releases have square corners.
const ROUNDED: f64 = 8.0;
const WAKE: u32 = WM_APP + 1;
const TIMER: usize = 1;

pub enum Command {
  /// Frame the window `target`.
  Frame { target: isize },
  /// Move the pointer to a screen point; `click` adds a ripple there.
  Pointer { at: (i32, i32), click: bool },
  /// Show one step of a batch with a clone that flies in to a screen point and leaves; `click` adds a ripple.
  Clone { at: (i32, i32), click: bool },
  Hide,
}

/// A clone on its way: its own window, its flight in, where it backs off to, when it set off.
struct Flight {
  window: isize,
  travel: Move,
  back: (f64, f64),
  born: Instant,
  click: bool,
}

/// Commands waiting for the main thread, and the window that wakes it.
static QUEUE: Mutex<Vec<Command>> = Mutex::new(Vec::new());
static WINDOW: AtomicIsize = AtomicIsize::new(0);

struct Overlay {
  frame: isize,
  pointer: isize,
  target: isize,
  /// The target's bounds and scale the frame was last drawn for.
  drawn: Option<(Rect, f64)>,
  scale: f64,
  /// The pointer's latest move and when it started; `None` once it was placed without one.
  travel: Option<(Move, Instant)>,
  /// Where the pointer was last sent.
  rest: (f64, f64),
  clones: Vec<Flight>,
  /// Clone windows not in use, kept for the next ones.
  spare: Vec<isize>,
  /// Clones sent so far, which picks the side the next one comes from.
  sent: usize,
  /// The pointer has been placed once: before that it appears where it is sent instead of gliding from a corner.
  placed: bool,
  /// When the ripple of the latest click starts.
  click: Option<Instant>,
  /// The scale the pointer's window shows a plain arrow at; `None` when it shows something else.
  arrow: Option<f64>,
  last: Instant,
  shown: bool,
  alpha: u8,
}

thread_local! {
  static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// Sends a command to the main thread. Callable from any thread.
pub fn send(command: Command) {
  if let Ok(mut queue) = QUEUE.lock() {
    queue.push(command);
  }
  let window = WINDOW.load(Ordering::Acquire);
  if window != 0 {
    // SAFETY: plain call; a window that is gone answers an error.
    let _ = unsafe { PostMessageW(Some(handle(window)), WAKE, WPARAM(0), LPARAM(0)) };
  }
}

/// Puts `bitmap` on a layered window with its top-left corner at `at`.
fn present(window: isize, bitmap: &Bitmap, at: (i32, i32), alpha: u8) {
  let info = BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: bitmap.width as i32, biHeight: -(bitmap.height as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
    ..Default::default()
  };
  // SAFETY: every object made here is released here; `bits` is the new bitmap's own memory, as large as the pixels
  // copied into it.
  unsafe {
    let screen = GetDC(None);
    let memory = CreateCompatibleDC(Some(screen));
    let mut bits: *mut c_void = std::ptr::null_mut();
    if let Ok(section) = CreateDIBSection(Some(screen), &raw const info, DIB_RGB_COLORS, &raw mut bits, None, 0)
      && !bits.is_null()
    {
      std::ptr::copy_nonoverlapping(bitmap.pixels.as_ptr(), bits.cast::<u32>(), bitmap.pixels.len());
      let previous = SelectObject(memory, section.into());
      let (place, size, origin) = (POINT { x: at.0, y: at.1 }, SIZE { cx: bitmap.width as i32, cy: bitmap.height as i32 }, POINT::default());
      let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
      let _ = UpdateLayeredWindow(handle(window), Some(screen), Some(&raw const place), Some(&raw const size), Some(memory), Some(&raw const origin), COLORREF(0), Some(&raw const blend), ULW_ALPHA);
      SelectObject(memory, previous);
      let _ = DeleteObject(section.into());
    }
    let _ = DeleteDC(memory);
    ReleaseDC(None, screen);
  }
}

/// Changes how see-through a layered window is, keeping its pixels.
fn dim(window: isize, alpha: u8) {
  let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
  // SAFETY: plain call; without a source the window keeps what it shows.
  let _ = unsafe { UpdateLayeredWindow(handle(window), None, None, None, None, None, COLORREF(0), Some(&raw const blend), ULW_ALPHA) };
}

fn above(window: isize) -> isize {
  // SAFETY: plain query; the top window has nothing above it and answers an error.
  unsafe { GetWindow(handle(window), GW_HWNDPREV) }.map(|above| above.0 as isize).unwrap_or(0)
}

/// The corner radius of the target window, in points.
fn radius(target: isize) -> f64 {
  let mut preference = 0u32;
  // SAFETY: the out-pointer is a live u32 of the size passed. Only Windows 11 knows the attribute, and only it
  // rounds corners; 1 is a window that asked for square ones.
  let rounded = unsafe { DwmGetWindowAttribute(handle(target), DWMWA_WINDOW_CORNER_PREFERENCE, (&raw mut preference).cast(), size_of::<u32>() as u32) }.is_ok() && preference != 1;
  // SAFETY: plain query.
  let zoomed = unsafe { IsZoomed(handle(target)).as_bool() };
  if rounded && !zoomed { ROUNDED } else { 0.0 }
}

impl Overlay {
  fn hide(&mut self) {
    // SAFETY: plain calls on this thread's own windows.
    unsafe {
      let _ = ShowWindow(handle(self.frame), SW_HIDE);
      let _ = ShowWindow(handle(self.pointer), SW_HIDE);
      let _ = KillTimer(Some(handle(self.frame)), TIMER);
    }
    for clone in std::mem::take(&mut self.clones) {
      // SAFETY: plain call on this thread's own window.
      let _ = unsafe { ShowWindow(handle(clone.window), SW_HIDE) };
      self.spare.push(clone.window);
    }
    self.shown = false;
    self.placed = false;
    self.drawn = None;
  }

  /// Puts the overlay's windows directly above the target in the stacking order: the frame, the pointer, then the
  /// clones.
  fn raise(&self) {
    let ours: Vec<isize> = [self.frame, self.pointer].into_iter().chain(self.clones.iter().map(|clone| clone.window)).collect();
    let mut over = above(self.target);
    while over != 0 && ours.contains(&over) {
      over = above(over);
    }
    // SAFETY: plain calls on this thread's own windows; the target's style is only read.
    unsafe {
      let topmost = GetWindowLongPtrW(handle(self.target), GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
      for window in ours {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        if over != 0 {
          let _ = SetWindowPos(handle(window), Some(handle(over)), 0, 0, 0, 0, flags);
        } else {
          // Nothing is above the target: the top of its own band, which for most windows is not the topmost one.
          let _ = SetWindowPos(handle(window), Some(if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, flags);
          let _ = SetWindowPos(handle(window), Some(if topmost { HWND_TOPMOST } else { HWND_TOP }), 0, 0, 0, 0, flags);
        }
      }
    }
  }

  fn show(&mut self) {
    self.last = Instant::now();
    if self.alpha != 255 {
      self.alpha = 255;
      dim(self.frame, 255);
      dim(self.pointer, 255);
    }
    if !self.shown {
      self.shown = true;
      // SAFETY: plain calls on this thread's own windows.
      unsafe {
        let _ = ShowWindow(handle(self.frame), SW_SHOWNOACTIVATE);
        SetTimer(Some(handle(self.frame)), TIMER, TICK, None);
      }
    }
  }

  fn run(&mut self, command: Command) {
    match command {
      Command::Frame { target } => {
        if target != self.target {
          self.target = target;
          self.drawn = None;
        }
        self.show();
        self.tick();
      }
      Command::Pointer { at, click } => {
        let to = (f64::from(at.0), f64::from(at.1));
        let now = Instant::now();
        // From where the pointer is right now, which is mid-flight when the last move has not ended.
        self.travel = self.placed.then(|| (Move::new(self.place(), to, self.scale), now));
        self.rest = to;
        self.click = click.then(|| now + self.travel.map_or(Duration::ZERO, |(travel, _)| Duration::from_secs_f64(travel.seconds)));
        if !self.placed {
          self.placed = true;
          self.arrow = None;
          // SAFETY: plain call on this thread's own window.
          let _ = unsafe { ShowWindow(handle(self.pointer), SW_SHOWNOACTIVATE) };
        }
        self.show();
        self.tick();
      }
      Command::Clone { at, click } => {
        self.show();
        self.clone_to((f64::from(at.0), f64::from(at.1)), click);
        self.tick();
      }
      Command::Hide => self.hide(),
    }
  }

  /// Sends a clone from outside the target window to `at`.
  fn clone_to(&mut self, at: (f64, f64), click: bool) {
    let Some(window) = self.spare.pop().or_else(layered) else { return };
    let start = motion::entry(frame_of(self.target), at, self.sent, self.scale);
    self.sent += 1;
    let clone = Flight { window, travel: Move::new(start, at, self.scale), back: motion::retreat(start, at, self.scale), born: Instant::now(), click };
    self.clones.push(clone);
    // SAFETY: plain call on this thread's own window.
    let _ = unsafe { ShowWindow(handle(window), SW_SHOWNOACTIVATE) };
    self.raise();
  }

  /// Where the pointer's tip is right now.
  fn place(&self) -> (f64, f64) {
    match self.travel {
      Some((travel, since)) => travel.at(since.elapsed().as_secs_f64() / travel.seconds),
      None => self.rest,
    }
  }

  /// Draws every clone where it is now; the ones that are done go back to the spares.
  fn draw_clones(&mut self, alpha: u8) {
    let scale = self.scale;
    let mut done = Vec::new();
    for (index, clone) in self.clones.iter().enumerate() {
      let t = clone.born.elapsed().as_secs_f64();
      let flight = clone.travel.seconds;
      let leave = flight + motion::CLONE_HOLD;
      let target = clone.travel.at(1.0);
      let (place, seen) = if t < flight {
        (clone.travel.at(t / flight), (t / motion::CLONE_APPEAR).min(1.0))
      } else if t < leave {
        (target, 1.0)
      } else if t < leave + motion::CLONE_LEAVE {
        let k = (t - leave) / motion::CLONE_LEAVE;
        let out = 1.0 - (1.0 - k) * (1.0 - k);
        ((target.0 + (clone.back.0 - target.0) * out, target.1 + (clone.back.1 - target.1) * out), 1.0 - k * k)
      } else {
        done.push(index);
        continue;
      };
      let ripple = (clone.click && t >= flight).then(|| (t - flight) / RIPPLE.as_secs_f64()).filter(|progress| *progress < 1.0);
      let corner = ((place.0 - paint::POINTER_TIP.0 * scale).round() as i32, (place.1 - paint::POINTER_TIP.1 * scale).round() as i32);
      present(clone.window, &paint::pointer(scale, ripple), corner, (f64::from(alpha) * seen).round() as u8);
    }
    for index in done.into_iter().rev() {
      let clone = self.clones.remove(index);
      // SAFETY: plain call on this thread's own window.
      let _ = unsafe { ShowWindow(handle(clone.window), SW_HIDE) };
      self.spare.push(clone.window);
    }
  }

  fn tick(&mut self) {
    if !self.shown {
      return;
    }
    // SAFETY: plain queries on the target, which may be gone: every one then answers false or zero.
    let (visible, dpi) = unsafe { (IsWindowVisible(handle(self.target)).as_bool(), GetDpiForWindow(handle(self.target))) };
    let idle = self.last.elapsed();
    if !visible || minimized(self.target) || idle >= IDLE + FADE {
      self.hide();
      return;
    }
    let alpha = if idle > IDLE { (255.0 * (1.0 - (idle - IDLE).as_secs_f64() / FADE.as_secs_f64())) as u8 } else { 255 };
    self.scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };

    // The frame follows the window: where it is, how large, and where it sits in the stack.
    let bounds = frame_of(self.target);
    let margin = (paint::FRAME_MARGIN * self.scale).ceil() as i32;
    let corner = (bounds.x as i32 - margin, bounds.y as i32 - margin);
    match self.drawn {
      Some((drawn, scale)) if (drawn.w, drawn.h, scale) == (bounds.w, bounds.h, self.scale) => {
        if (drawn.x, drawn.y) != (bounds.x, bounds.y) {
          // SAFETY: plain call on this thread's own window.
          let _ = unsafe { SetWindowPos(handle(self.frame), None, corner.0, corner.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
          self.drawn = Some((bounds, self.scale));
        }
      }
      _ => {
        present(self.frame, &paint::frame((bounds.w.max(1.0) as usize, bounds.h.max(1.0) as usize), radius(self.target), self.scale), corner, alpha);
        self.drawn = Some((bounds, self.scale));
      }
    }
    if above(self.target) != self.frame || (self.placed && above(self.frame) != self.pointer) {
      self.raise();
    }
    if alpha != self.alpha {
      self.alpha = alpha;
      dim(self.frame, alpha);
      dim(self.pointer, alpha);
    }
    self.draw_clones(alpha);

    if self.placed {
      let (x, y) = self.place();
      let corner = ((x - paint::POINTER_TIP.0 * self.scale).round() as i32, (y - paint::POINTER_TIP.1 * self.scale).round() as i32);
      let ripple = self.click.and_then(|start| Instant::now().checked_duration_since(start)).map(|since| since.as_secs_f64() / RIPPLE.as_secs_f64());
      match ripple {
        Some(progress) if progress < 1.0 => {
          present(self.pointer, &paint::pointer(self.scale, Some(progress)), corner, alpha);
          self.arrow = None;
        }
        _ => {
          if ripple.is_some() {
            self.click = None;
            self.arrow = None;
          }
          if self.arrow != Some(self.scale) {
            present(self.pointer, &paint::pointer(self.scale, None), corner, alpha);
            self.arrow = Some(self.scale);
          } else {
            // SAFETY: plain call on this thread's own window.
            let _ = unsafe { SetWindowPos(handle(self.pointer), None, corner.0, corner.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
          }
        }
      }
    }
  }
}

unsafe extern "system" fn on_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
  match message {
    WAKE => {
      let commands: Vec<Command> = QUEUE.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default();
      OVERLAY.with(|slot| {
        if let Some(overlay) = slot.borrow_mut().as_mut() {
          for command in commands {
            overlay.run(command);
          }
        }
      });
      LRESULT(0)
    }
    WM_TIMER => {
      OVERLAY.with(|slot| {
        if let Some(overlay) = slot.borrow_mut().as_mut() {
          overlay.tick();
        }
      });
      LRESULT(0)
    }
    // Clicks fall through to whatever is underneath.
    WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
    // SAFETY: the arguments are the ones this procedure was called with.
    _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
  }
}

/// A new overlay window: layered, click-through, never active, not in the task bar. Main thread only, once the
/// window class is registered (`run_main`).
fn layered() -> Option<isize> {
  let style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
  // SAFETY: plain window calls with their declared arguments; the window lives as long as the process.
  unsafe {
    let instance = GetModuleHandleW(None).map(|module| module.into()).ok();
    CreateWindowExW(style, w!("AcpiraComputerOverlay"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, instance, None).ok().map(|window| window.0 as isize).filter(|window| *window != 0)
  }
}

/// Runs the overlay on the calling thread, which must be the process's main thread. Never returns: the process ends
/// when its input does.
pub fn run_main() -> ! {
  // SAFETY: plain window calls with their declared arguments; the class and both windows live as long as the process.
  unsafe {
    let instance = GetModuleHandleW(None).map(|module| module.into()).unwrap_or_default();
    let class = WNDCLASSW { lpfnWndProc: Some(on_message), hInstance: instance, lpszClassName: w!("AcpiraComputerOverlay"), ..Default::default() };
    RegisterClassW(&raw const class);
    if let (Some(frame), Some(pointer)) = (layered(), layered()) {
      let now = Instant::now();
      OVERLAY.with(|slot| {
        *slot.borrow_mut() = Some(Overlay {
          frame,
          pointer,
          target: 0,
          drawn: None,
          scale: 1.0,
          travel: None,
          rest: (0.0, 0.0),
          clones: Vec::new(),
          spare: Vec::new(),
          sent: 0,
          placed: false,
          click: None,
          arrow: None,
          last: now,
          shown: false,
          alpha: 255,
        });
      });
      WINDOW.store(frame, Ordering::Release);
      // Whatever was sent before the window existed.
      let _ = PostMessageW(Some(handle(frame)), WAKE, WPARAM(0), LPARAM(0));
    }
    let mut message = MSG::default();
    while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
      DispatchMessageW(&raw const message);
    }
  }
  std::process::exit(0)
}
