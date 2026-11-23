use anyhow::Result;
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
};
use tokio_util::sync::CancellationToken;

unsafe extern "C" {
    fn avatar3d_run_window(
        url: *const c_char,
        context: *const c_void,
        request_quit: extern "C" fn(*const c_void),
    );
    fn avatar3d_service_ended();
    fn avatar3d_startup_error(message: *const c_char);
}

extern "C" fn request_quit(context: *const c_void) {
    // AppKit calls this on the main thread only while the owning window is running.
    unsafe { &*context.cast::<CancellationToken>() }.cancel();
}

pub fn show(port: u16, admin: &str, quit: Option<&CancellationToken>) -> Result<()> {
    let url = CString::new(format!("http://127.0.0.1:{port}/#{admin}"))?;
    let context = quit.map_or(ptr::null(), ptr::from_ref).cast();
    // The Cocoa event loop stays on the main thread. Both borrows outlive this call.
    unsafe { avatar3d_run_window(url.as_ptr(), context, request_quit) };
    Ok(())
}

pub fn service_ended() {
    // The native side dispatches this notification onto the main queue.
    unsafe { avatar3d_service_ended() };
}

pub fn startup_error(error: &anyhow::Error) {
    if let Ok(message) = CString::new(format!("{error:#}")) {
        unsafe { avatar3d_startup_error(message.as_ptr()) };
    }
}
