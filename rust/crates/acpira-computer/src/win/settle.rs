//! Listens to apps' accessibility events: one thread holds an event hook per app and stamps every event
//! (`crate::settle`). The hooks are out-of-context ones, so nothing is loaded into the apps; they deliver to the
//! thread that set them, which therefore runs a message loop.

use crate::settle::Stamp;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, EVENT_OBJECT_END, EVENT_SYSTEM_SOUND, GetMessageW, MSG, OBJID_CARET, OBJID_CURSOR, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP};

/// Thread messages: hook the process in `wParam`, unhook it.
const HOOK: u32 = WM_APP + 1;
const UNHOOK: u32 = WM_APP + 2;

type Stamps = Arc<Mutex<HashMap<u32, Arc<Stamp>>>>;

thread_local! {
  /// The listening thread's hooks, with the stamp each one writes.
  static HOOKS: RefCell<HashMap<isize, (u32, Arc<Stamp>)>> = RefCell::new(HashMap::new());
}

pub struct Watcher {
  /// The listening thread; 0 when it could not start, and then nothing is ever seen.
  thread: u32,
  stamps: Stamps,
}

unsafe extern "system" fn on_event(hook: HWINEVENTHOOK, _event: u32, _window: HWND, object: i32, _child: i32, _thread: u32, _time: u32) {
  // The pointer's moves and the caret's blinking are not the interface changing.
  if object == OBJID_CURSOR.0 || object == OBJID_CARET.0 {
    return;
  }
  HOOKS.with(|hooks| {
    if let Some((_, stamp)) = hooks.borrow().get(&(hook.0 as isize)) {
      stamp.hit();
    }
  });
}

fn listen(stamps: &Stamps, ready: &std::sync::mpsc::Sender<u32>) {
  let mut message = MSG::default();
  // SAFETY: plain calls on this thread's own message queue, which the first one creates; the hooks set here are
  // removed here.
  unsafe {
    let _ = PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE);
    let _ = ready.send(GetCurrentThreadId());
    while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
      let pid = message.wParam.0 as u32;
      match message.message {
        HOOK => {
          let Some(stamp) = stamps.lock().ok().and_then(|stamps| stamps.get(&pid).cloned()) else { continue };
          let hook = SetWinEventHook(EVENT_SYSTEM_SOUND, EVENT_OBJECT_END, None, Some(on_event), pid, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
          if !hook.is_invalid() {
            HOOKS.with(|hooks| hooks.borrow_mut().insert(hook.0 as isize, (pid, stamp)));
          }
        }
        UNHOOK => HOOKS.with(|hooks| {
          hooks.borrow_mut().retain(|hook, (hooked, _)| {
            if *hooked == pid {
              let _ = UnhookWinEvent(HWINEVENTHOOK(*hook as *mut _));
            }
            *hooked != pid
          });
        }),
        // The events themselves arrive as calls made from inside the dispatch.
        _ => {
          DispatchMessageW(&raw const message);
        }
      }
    }
  }
}

impl Watcher {
  pub fn start() -> Watcher {
    let stamps: Stamps = Arc::default();
    let (ready, wait) = std::sync::mpsc::channel();
    let shared = stamps.clone();
    let spawned = std::thread::Builder::new().name("event-watch".into()).spawn(move || listen(&shared, &ready));
    let thread = match spawned {
      Ok(_) => wait.recv_timeout(Duration::from_millis(500)).unwrap_or(0),
      Err(_) => 0,
    };
    Watcher { thread, stamps }
  }

  fn post(&self, message: u32, pid: u32) {
    if self.thread != 0 {
      // SAFETY: plain call; a thread that is gone answers an error.
      let _ = unsafe { PostThreadMessageW(self.thread, message, WPARAM(pid as usize), LPARAM(0)) };
    }
  }

  /// The stamp of `pid`'s events, listening from now on.
  pub fn watch(&self, pid: u32) -> Arc<Stamp> {
    let Ok(mut stamps) = self.stamps.lock() else { return Arc::default() };
    if let Some(stamp) = stamps.get(&pid) {
      return stamp.clone();
    }
    let stamp: Arc<Stamp> = Arc::default();
    stamps.insert(pid, stamp.clone());
    drop(stamps);
    self.post(HOOK, pid);
    stamp
  }

  /// Stops listening to the processes `keep` does not name.
  pub fn retain(&self, keep: impl Fn(u32) -> bool) {
    let Ok(mut stamps) = self.stamps.lock() else { return };
    let gone: Vec<u32> = stamps.keys().copied().filter(|pid| !keep(*pid)).collect();
    for pid in gone {
      stamps.remove(&pid);
      self.post(UNHOOK, pid);
    }
  }
}
