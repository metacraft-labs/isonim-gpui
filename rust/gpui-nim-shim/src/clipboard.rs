//! PLAT-50 (CodeTracer) — **THE SYSTEM CLIPBOARD, FOR A CLICK THAT COPIES.**
//!
//! A front-end copies on a user's click (an editor menu's "Copy", a status
//! bar location), on its own thread and outside any GPUI context, so the text
//! is handed over here and GPUI's platform clipboard takes it on the root
//! view's next frame (`NimRootView::render` calls `take_pending`), which is
//! the one place an `App` is in hand. The text last written is kept too, and
//! is what `gpui_clipboard_text` answers — so a build without a window (a
//! render plan, a headless test) can still say what it was asked to copy.
//!
//! Feature-less, for `input`'s reason: the exported symbol set does not
//! depend on the features the cdylib was built with.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;

static PENDING: Mutex<Option<String>> = Mutex::new(None);
static LAST: Mutex<String> = Mutex::new(String::new());

/// Hand `text` to the platform clipboard on the next frame.
pub fn write(text: &str) {
    *PENDING.lock().unwrap_or_else(|p| p.into_inner()) = Some(text.to_string());
    *LAST.lock().unwrap_or_else(|p| p.into_inner()) = text.to_string();
    crate::window::request_repaint();
}

/// The text written since the last frame, if any (taken: a frame writes it
/// to the platform clipboard once).
pub fn take_pending() -> Option<String> {
    PENDING.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// The text most recently written.
pub fn last() -> String {
    LAST.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// `gpui_write_clipboard(text)`: copy the NUL-terminated UTF-8 `text`.
#[no_mangle]
pub extern "C" fn gpui_write_clipboard(text: *const c_char) {
    if text.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    write(&s);
}

/// `gpui_clipboard_text(buf, buf_len)`: the text most recently written,
/// NUL-terminated into `buf`; answers the length it needs (call with a null
/// `buf` to ask), as `gpui_get_text_content` does.
#[no_mangle]
pub extern "C" fn gpui_clipboard_text(buf: *mut u8, buf_len: u64) -> u64 {
    let text = last();
    let bytes = text.as_bytes();
    let needed = bytes.len() as u64;
    if buf.is_null() || buf_len == 0 {
        return needed;
    }
    let to_copy = std::cmp::min(bytes.len(), (buf_len - 1) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, to_copy);
        *buf.add(to_copy) = 0;
    }
    needed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_keeps_the_text_and_one_pending_copy() {
        write("main.py:31  tick 32");
        assert_eq!(last(), "main.py:31  tick 32");
        assert_eq!(take_pending().as_deref(), Some("main.py:31  tick 32"));
        // Taken: the next frame does not write it again.
        assert_eq!(take_pending(), None);
        assert_eq!(last(), "main.py:31  tick 32");
        let c = std::ffi::CString::new("return left + right\n").unwrap();
        gpui_write_clipboard(c.as_ptr());
        let mut buf = vec![0u8; 64];
        let n = gpui_clipboard_text(buf.as_mut_ptr(), buf.len() as u64);
        assert_eq!(n, 20);
        assert_eq!(&buf[..20], b"return left + right\n");
        assert_eq!(gpui_clipboard_text(std::ptr::null_mut(), 0), 20);
    }
}
