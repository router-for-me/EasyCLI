//! Keep activation changes from repainting the hidden native caption.
//!
//! Tao preserves WS_CAPTION on undecorated windows. Forwarding WM_NCACTIVATE
//! with lParam = -1 suppresses DefWindowProc's nonclient repaint while retaining
//! Tao's activation/focus handling. This avoids one theme-hook entry point; it
//! does not suppress nonclient clicks, system commands, or resize messages.

use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{GetWindowThreadProcessId, WM_NCACTIVATE, WM_NCDESTROY},
    },
};

const SUBCLASS_ID: usize = 0x4543_5041;

pub(crate) fn install(window: &tauri::WebviewWindow) -> io::Result<()> {
    let hwnd = window.hwnd().map_err(io::Error::other)?.0 as HWND;
    // Called from setup on the owning UI thread, before removing decorations.
    unsafe { install_on_window(hwnd) }
}

// The caller must keep this window alive on its owning thread.
unsafe fn install_on_window(hwnd: HWND) -> io::Result<()> {
    if GetWindowThreadProcessId(hwnd, ptr::null_mut()) != GetCurrentThreadId() {
        return Err(io::Error::other(
            "The Windows caption guard must be installed on the window's owning thread",
        ));
    }
    if SetWindowSubclass(hwnd, Some(caption_guard), SUBCLASS_ID, 0) == 0 {
        // SetWindowSubclass does not document GetLastError, so do not report a
        // stale last-error value or silently continue without the guard.
        return Err(io::Error::other(
            "Failed to install the Windows caption guard",
        ));
    }
    Ok(())
}

unsafe extern "system" fn caption_guard(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        RemoveWindowSubclass(hwnd, Some(caption_guard), SUBCLASS_ID);
    }
    // Do not consume activation: Tao must still update its focus state.
    let lparam = if message == WM_NCACTIVATE { -1 } else { lparam };
    DefSubclassProc(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use windows_sys::Win32::UI::Shell::GetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, WM_APP, WM_SYSCOMMAND, WS_POPUP,
    };

    thread_local! {
        static REMOVED: Cell<bool> = const { Cell::new(false) };
        static RECEIVED: Cell<(u32, WPARAM, LPARAM)> = const { Cell::new((0, 0, 0)) };
    }

    unsafe extern "system" fn observer(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if matches!(message, WM_NCACTIVATE | WM_SYSCOMMAND | WM_APP) {
            RECEIVED.with(|received| received.set((message, wparam, lparam)));
            return 123;
        }
        if message == WM_NCDESTROY {
            let mut data = 0;
            REMOVED.with(|removed| {
                removed
                    .set(GetWindowSubclass(hwnd, Some(caption_guard), SUBCLASS_ID, &mut data) == 0)
            });
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    struct TestWindow(HWND);

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn forwards_activation_and_preserves_other_window_messages() {
        // A hidden, isolated Win32 window: no Tauri app, core process or user data.
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let window = TestWindow(unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                100,
                100,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        });
        assert!(!window.0.is_null());
        unsafe {
            assert_ne!(SetWindowSubclass(window.0, Some(observer), 1, 0), 0);
            install_on_window(window.0).unwrap();
            // Reinstallation updates the same subclass instead of stacking it.
            install_on_window(window.0).unwrap();
            for active in [0, 1] {
                assert_eq!(SendMessageW(window.0, WM_NCACTIVATE, active, 42), 123);
                assert_eq!(RECEIVED.with(Cell::get), (WM_NCACTIVATE, active, -1));
            }
            for message in [WM_SYSCOMMAND, WM_APP] {
                assert_eq!(SendMessageW(window.0, message, 7, 42), 123);
                assert_eq!(RECEIVED.with(Cell::get), (message, 7, 42));
            }
            drop(window);
            assert!(
                REMOVED.with(Cell::get),
                "guard removed before forwarding destruction"
            );
        }
    }
}
