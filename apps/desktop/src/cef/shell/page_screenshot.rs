//! A screenshot of the top of a page a windowed browser is showing, taken over the DevTools
//! protocol (`Page.captureScreenshot`) the appearance and keep-awake code already drive. Used for
//! the chat's page card pictures (`app/visual_page_thumbnails.rs`).
//!
//! CDXC:CefRuntime 2026-10-11 WHY:
//! The picture comes only from a page the user already has open on screen (the floating page window). No hidden or off-screen browser is made for it and the runtime is never started for it, so no other web view pays anything; an earlier off-screen capture that needed CEF's windowless rendering switched on was rejected by the user. The answer arrives on the observer while CEF's message pump runs, which can be inside a gpui update, so it only fills a channel.
use super::*;
use cef::{
    DevToolsMessageObserver, ImplDevToolsMessageObserver, WrapDevToolsMessageObserver,
    wrap_dev_tools_message_observer,
};
use futures::channel::oneshot;

/// The answer slot of one screenshot request.
type ScreenshotSlot = Arc<Mutex<Option<(c_int, oneshot::Sender<Option<String>>)>>>;

/// A screenshot on its way. Keep it until `picture` resolves: dropping it stops listening.
pub struct PageScreenshot {
    /// Base64 PNG, or `None` when the page refused.
    pub picture: oneshot::Receiver<Option<String>>,
    _registration: Option<cef::Registration>,
}

wrap_dev_tools_message_observer! {
    struct PageScreenshotObserver {
        slot: ScreenshotSlot,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            message_id: c_int,
            success: c_int,
            result: Option<&[u8]>,
        ) {
            let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if slot.as_ref().map(|(id, _)| *id) != Some(message_id) {
                return;
            }
            let Some((_, sender)) = slot.take() else {
                return;
            };
            let picture = (success != 0)
                .then_some(result)
                .flatten()
                .and_then(|result| serde_json::from_slice::<serde_json::Value>(result).ok())
                .and_then(|value| value.get("data")?.as_str().map(str::to_string));
            let _ = sender.send(picture);
        }
    }
}

impl CefBrowser {
    /// Asks the page for a PNG of the area `width`×`height` CSS pixels at its top left, drawn at
    /// `scale`. `None` when the request could not be sent.
    pub fn capture_page_top(&self, width: f64, height: f64, scale: f64) -> Option<PageScreenshot> {
        let browser = self.browser.borrow().clone();
        let host = browser.host()?;
        let (sender, picture) = oneshot::channel();
        let slot: ScreenshotSlot = Arc::new(Mutex::new(None));
        let mut observer = PageScreenshotObserver::new(slot.clone());
        let registration = host.add_dev_tools_message_observer(Some(&mut observer));
        let mut clip = cef::dictionary_value_create()?;
        for (key, value) in [
            ("x", 0.0),
            ("y", 0.0),
            ("width", width),
            ("height", height),
            ("scale", scale),
        ] {
            clip.set_double(Some(&CefString::from(key)), value);
        }
        let mut params = cef::dictionary_value_create()?;
        params.set_string(
            Some(&CefString::from("format")),
            Some(&CefString::from("png")),
        );
        params.set_dictionary(Some(&CefString::from("clip")), Some(&mut clip));
        let message_id = next_page_appearance_devtools_message_id();
        // Filled before the call: CEF may answer while it executes.
        *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((message_id, sender));
        let sent = host.execute_dev_tools_method(
            message_id,
            Some(&CefString::from("Page.captureScreenshot")),
            Some(&mut params),
        ) != 0;
        sent.then_some(PageScreenshot {
            picture,
            _registration: registration,
        })
    }
}
