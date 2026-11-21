//! Runs the browser pane's page script on Linux through WebKitGTK's `call_async_javascript_function`
//! (WebKitGTK 2.40 or later), which awaits the script's promise and hands back its value.

use javascriptcore::ValueExt;
use webkit2gtk::{WebView, WebViewExt, gio};

/// Calls `body` as the body of an async function in the page and passes its string result, or the
/// error it threw, to `finish`. Must run on the main thread, which `with_webview` provides.
pub fn call(webview: &WebView, body: &str, finish: Box<dyn FnOnce(Result<String, String>) + Send>) {
    webview.call_async_javascript_function(
        body,
        None,
        None,
        None,
        None::<&gio::Cancellable>,
        move |result| {
            finish(
                result
                    .map(|value| value.to_str().to_string())
                    .map_err(|error| error.message().to_string()),
            )
        },
    );
}
