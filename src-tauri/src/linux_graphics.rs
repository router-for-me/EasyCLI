//! Apply Linux graphics compatibility defaults before GTK/WebKit initialization.
//!
//! WebKit's DMA-BUF path can fail with GBM buffer errors, while native Wayland
//! can terminate GTK with a protocol error (#406). Prefer X11 when advertised,
//! but let GTK fall back to Wayland if that display cannot actually be opened.

use std::{env, ffi::OsString};

fn defaults(get: impl Fn(&str) -> Option<OsString>) -> Vec<(&'static str, &'static str)> {
    let mut values = Vec::new();
    if get("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        values.push(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
    }
    if get("GDK_BACKEND").is_none() && get("DISPLAY").is_some_and(|value| !value.is_empty()) {
        values.push(("GDK_BACKEND", "x11,wayland"));
    }
    values
}

/// # Safety
/// Must run on the main thread before any other threads or GUI libraries start.
pub(super) unsafe fn configure() {
    for (key, value) in defaults(|key| env::var_os(key)) {
        // SAFETY: the caller guarantees single-threaded startup.
        unsafe { env::set_var(key, value) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(environment: &[(&str, &str)]) -> Vec<(&'static str, &'static str)> {
        defaults(|key| {
            environment
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn x11_and_xwayland_sessions_get_both_workarounds_without_arguments() {
        for environment in [
            vec![("DISPLAY", ":0")],
            vec![("DISPLAY", ":1"), ("WAYLAND_DISPLAY", "wayland-0")],
        ] {
            assert_eq!(
                settings(&environment),
                vec![
                    ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
                    ("GDK_BACKEND", "x11,wayland"),
                ]
            );
        }
    }

    #[test]
    fn pure_wayland_and_headless_sessions_do_not_force_x11() {
        for environment in [
            vec![],
            vec![("WAYLAND_DISPLAY", "wayland-0")],
            vec![("DISPLAY", ""), ("WAYLAND_DISPLAY", "wayland-0")],
        ] {
            assert_eq!(
                settings(&environment),
                vec![("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]
            );
        }
    }

    #[test]
    fn explicit_backend_does_not_disable_the_independent_webkit_fix() {
        assert_eq!(
            settings(&[("DISPLAY", ":0"), ("GDK_BACKEND", "wayland")]),
            vec![("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]
        );
    }

    #[test]
    fn explicit_renderer_does_not_disable_the_independent_backend_fix() {
        assert_eq!(
            settings(&[("DISPLAY", ":0"), ("WEBKIT_DISABLE_DMABUF_RENDERER", "0")]),
            vec![("GDK_BACKEND", "x11,wayland")]
        );
    }

    #[test]
    fn explicit_overrides_and_inherited_defaults_are_preserved() {
        for (backend, renderer) in [("wayland", "0"), ("x11,wayland", "1"), ("", "")] {
            assert!(settings(&[
                ("DISPLAY", ":0"),
                ("GDK_BACKEND", backend),
                ("WEBKIT_DISABLE_DMABUF_RENDERER", renderer),
            ])
            .is_empty());
        }
    }
}
