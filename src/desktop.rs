use anyhow::Result;
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
};
use tokio_util::sync::CancellationToken;

unsafe extern "C" {
    fn vorela_run_window(
        url: *const c_char,
        context: *const c_void,
        request_quit: extern "C" fn(*const c_void),
    );
    fn vorela_service_ended();
    fn vorela_startup_error(message: *const c_char, english: *const c_char);
}

extern "C" fn request_quit(context: *const c_void) {
    // AppKit calls this on the main thread only while the owning window is running.
    unsafe { &*context.cast::<CancellationToken>() }.cancel();
}

pub fn show(port: u16, admin: &str, quit: Option<&CancellationToken>) -> Result<()> {
    let url = CString::new(format!("http://127.0.0.1:{port}/#{admin}"))?;
    let context = quit.map_or(ptr::null(), ptr::from_ref).cast();
    // The Cocoa event loop stays on the main thread. Both borrows outlive this call.
    unsafe { vorela_run_window(url.as_ptr(), context, request_quit) };
    Ok(())
}

pub fn service_ended() {
    // The native side dispatches this notification onto the main queue.
    unsafe { vorela_service_ended() };
}

fn english_startup_message(error: &anyhow::Error) -> String {
    let translations: std::collections::HashMap<String, String> =
        serde_json::from_str(include_str!("../web/i18n-en.json")).expect("English dictionary");
    error
        .chain()
        .map(|cause| {
            let text = cause.to_string();
            translations.get(&text).cloned().unwrap_or(text)
        })
        .collect::<Vec<_>>()
        .join(": ")
}

pub fn startup_error(error: &anyhow::Error) {
    let english = english_startup_message(error);
    if let (Ok(message), Ok(english)) = (CString::new(format!("{error:#}")), CString::new(english))
    {
        unsafe { vorela_startup_error(message.as_ptr(), english.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn localizes_startup_context_and_preserves_the_underlying_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = std::fs::File::open(dir.path().join("studio.lock")).unwrap_err();
        let os_message = missing.to_string();
        let error = anyhow::Error::new(missing).context("该端口上的 Vorela 使用其他数据目录");
        assert_eq!(
            super::english_startup_message(&error),
            format!(
                "The Vorela instance on this port is using a different data folder: {os_message}"
            )
        );
    }
}
