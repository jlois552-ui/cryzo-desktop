//! Runs the browser pane's page script on macOS through WKWebView's `callAsyncJavaScript`, which
//! awaits the script's promise and hands back its value.

use std::{cell::Cell, ffi::c_void};

use block2::RcBlock;
use objc2::{MainThreadMarker, runtime::AnyObject};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};

/// Calls `body` as the body of an async function in the page and passes its string result, or the
/// error it threw, to `finish`. Must run on the main thread, which `with_webview` provides.
pub fn call(
    webview: *mut c_void,
    body: &str,
    finish: Box<dyn FnOnce(Result<String, String>) + Send>,
) {
    let Some(main) = MainThreadMarker::new() else {
        finish(Err(
            "the browser pane must be called on the main thread".into()
        ));
        return;
    };
    // SAFETY: Tauri's `PlatformWebview::inner` is the pane's live WKWebView, borrowed for this call.
    let Some(webview) = (unsafe { webview.cast::<WKWebView>().as_ref() }) else {
        finish(Err("the browser pane isn't open".into()));
        return;
    };
    let finish = Cell::new(Some(finish));
    let handler = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        let Some(finish) = finish.take() else {
            return;
        };
        // SAFETY: WebKit passes a live NSError or null.
        if let Some(error) = unsafe { error.as_ref() } {
            finish(Err(error.localizedDescription().to_string()));
            return;
        }
        // SAFETY: a live object or null; the script returns a JSON string.
        let text = unsafe { result.as_ref() }
            .and_then(|object| object.downcast_ref::<NSString>())
            .map(NSString::to_string);
        finish(text.ok_or_else(|| "the page returned nothing".to_string()));
    });
    // SAFETY: main-thread WebKit calls on a live web view; the block outlives the call because
    // WebKit copies it.
    unsafe {
        webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(body),
            None,
            None,
            &WKContentWorld::pageWorld(main),
            Some(&handler),
        );
    }
}
