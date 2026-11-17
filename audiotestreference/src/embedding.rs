//! X11 reparenting adapted from nam-linux-port (GPL-3.0-or-later).
use std::time::Duration;
use x11rb::{connection::Connection, protocol::xproto::ConnectionExt};

pub fn embed(
    parent: u32,
    scale: f32,
) -> Result<x11rb::rust_connection::RustConnection, Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    // GPUI 0.2.2's X11 HasWindowHandle implementation panics. Locate our sole
    // window by its process property instead, including any WM frame ancestors.
    let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let mut pending = vec![connection.setup().roots[screen].root];
    let mut child = None;
    while let Some(window) = pending.pop() {
        if let Ok(property) = connection
            .get_property(
                false,
                window,
                pid_atom,
                x11rb::protocol::xproto::AtomEnum::CARDINAL,
                0,
                1,
            )?
            .reply()
        {
            if property.value32().and_then(|mut values| values.next()) == Some(std::process::id()) {
                child = Some(window);
                break;
            }
        }
        if let Ok(tree) = connection.query_tree(window)?.reply() {
            pending.extend(tree.children);
        }
    }
    let child = child.ok_or("Cannot find the GPUI X11 window for this process")?;
    // GPUI maps before exposing the view. Let the desktop WM withdraw it
    // before changing it into an unmanaged child of the plugin host.
    connection.unmap_window(child)?.check()?;
    connection.flush()?;
    std::thread::sleep(Duration::from_millis(100));
    connection
        .change_window_attributes(
            child,
            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().override_redirect(1),
        )?
        .check()?;
    connection
        .configure_window(
            child,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .width((660. * scale).round() as u32)
                .height((430. * scale).round() as u32),
        )?
        .check()?;
    // A WM may still process the original top-level MapRequest and reparent
    // back to root. Retry until its pending management requests have settled.
    for _ in 0..50 {
        connection.reparent_window(child, parent, 0, 0)?.check()?;
        connection.map_window(child)?.check()?;
        connection.flush()?;
        std::thread::sleep(Duration::from_millis(20));
        if connection.query_tree(child)?.reply()?.parent == parent {
            return Ok(connection);
        }
    }
    Err("The window manager prevented embedding the editor".into())
}
