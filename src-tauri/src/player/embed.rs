//! Keeping the video underneath the controls.
//!
//! mpv is given our player window's `HWND` and creates its own child window
//! inside it. That child is created *after* the WebView2 child, so Windows puts
//! it on top and it hides the interface. Pushing it to the bottom of the
//! z-order — the webview above it, its background transparent — is what lets
//! our own controls sit over the picture.
//!
//! mpv creates that child asynchronously, so this runs a few times after
//! playback starts rather than once.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, HWND_BOTTOM, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetWindowPos,
};

/// Window class mpv registers for its video output.
const MPV_CLASS: &str = "mpv";

/// Sends every mpv child of `parent` to the back.
///
/// Returns how many were moved, so the caller can stop retrying once mpv's
/// window has actually appeared.
pub fn push_video_to_back(parent: isize) -> usize {
    let mut moved = 0usize;
    let parent = HWND(parent as *mut _);

    // SAFETY: `parent` is a live window handle owned by this process, and the
    // callback only reads class names and reorders windows.
    unsafe {
        let _ = EnumChildWindows(
            Some(parent),
            Some(enum_child),
            LPARAM(&mut moved as *mut usize as isize),
        );
    }
    moved
}

unsafe extern "system" fn enum_child(child: HWND, counter: LPARAM) -> BOOL {
    let mut buffer = [0u16; 64];
    // SAFETY: `child` comes from EnumChildWindows and the buffer is ours.
    let written = unsafe { GetClassNameW(child, &mut buffer) };
    if written > 0 {
        let class = String::from_utf16_lossy(&buffer[..written as usize]);
        if class == MPV_CLASS {
            // SAFETY: reordering a child window of our own window.
            let ok = unsafe {
                SetWindowPos(
                    child,
                    Some(HWND_BOTTOM),
                    0,
                    0,
                    0,
                    0,
                    SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0),
                )
            };
            if ok.is_ok() {
                // SAFETY: the pointer was handed in by `push_video_to_back`.
                unsafe {
                    let counter = counter.0 as *mut usize;
                    *counter += 1;
                }
            }
        }
    }
    // Keep enumerating: a window can hold more than one mpv surface.
    BOOL::from(true)
}
