use crate::app::helpers::*;
use gpui::{Image, ImageFormat};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

#[derive(Default)]
struct ImageCache {
    images: HashMap<String, Arc<Image>>,
    order: VecDeque<String>,
    source_bytes: usize,
}

thread_local! {
    static IMAGES: RefCell<[ImageCache; 2]> = RefCell::default();
}

pub(crate) fn sidebar_image(value: &str) -> Option<Arc<Image>> {
    image(value, false)
}

pub(crate) fn agent_image(value: &str, agent: Option<&str>, light: bool) -> Option<Arc<Image>> {
    image(
        value,
        light
            && matches!(
                agent,
                Some(
                    "codex"
                        | "copilot"
                        | "amp-cli"
                        | "cursor-cli"
                        | "grok-build"
                        | "mastra"
                        | "mini-swe-agent"
                        | "zcode"
                        | "freebuff"
                )
            ),
    )
}

fn image(value: &str, monochrome: bool) -> Option<Arc<Image>> {
    let value = value.trim();
    if value.len() > 4 * 1024 * 1024 {
        return None;
    }
    // CDXC:Sidebar 2026-09-17 WHY:
    // Scroll and spinner frames reuse the same icon data; decoding and hashing new GPUI images on every row render wastes UI-thread time.
    // Bound retained source data and entries independently, including separate light-theme artwork.
    IMAGES.with_borrow_mut(|caches| {
        let cache = &mut caches[usize::from(monochrome)];
        if let Some(image) = cache.images.get(value) {
            return Some(image.clone());
        }
        let image = decode_image(value, monochrome)?;
        while cache.images.len() >= 128 || cache.source_bytes + value.len() > 4 * 1024 * 1024 {
            let oldest = cache.order.pop_front()?;
            cache.source_bytes -= oldest.len();
            cache.images.remove(&oldest);
        }
        cache.source_bytes += value.len();
        cache.order.push_back(value.to_owned());
        cache.images.insert(value.to_owned(), image.clone());
        Some(image)
    })
}

fn decode_image(value: &str, monochrome: bool) -> Option<Arc<Image>> {
    let (metadata, payload) = value.strip_prefix("data:")?.split_once(',')?;
    let (format, encoded) = if metadata.split(';').next()? == "image/svg+xml" {
        (
            ImageFormat::Svg,
            metadata
                .split(';')
                .any(|part| part.eq_ignore_ascii_case("base64")),
        )
    } else {
        browser_favicon_data_url_metadata(metadata)?
    };
    let bytes = browser_favicon_percent_decode(payload, 4 * 1024 * 1024)?;
    let bytes = if encoded {
        browser_favicon_decode_base64(&bytes)?
    } else {
        bytes
    };
    if bytes.is_empty() {
        return None;
    }
    let bytes = if monochrome && format == ImageFormat::Svg {
        let mut svg = String::from_utf8(bytes).ok()?;
        let start = svg.find("<svg")?;
        let content = start + svg[start..].find('>')? + 1;
        let end = svg.rfind("</svg>")?;
        svg.insert_str(end, "</g>");
        svg.insert_str(content, r##"<defs><filter id="native-sidebar-monochrome" color-interpolation-filters="sRGB"><feColorMatrix type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0"/></filter></defs><g filter="url(#native-sidebar-monochrome)">"##);
        svg.into_bytes()
    } else {
        bytes
    };
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
