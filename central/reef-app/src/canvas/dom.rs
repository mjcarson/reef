//! What the canvas asks of the browser, and nothing on the server, where nothing is drawn
//!
//! Dioxus draws the canvas and hears its events. What it can't do is here: an element's place on
//! the page, pointer capture, local storage, timers, full screen, text measurement, reading a
//! doc's page, and the classes and overlays inside the charts' SVG, which come in as markup.
//! The server renders the page before any plan is read, so its versions do nothing.

#[cfg(feature = "web")]
pub use web::*;

#[cfg(not(feature = "web"))]
pub use server::*;

/// An element's box on the page, in CSS pixels
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    /// Its left edge, from the viewport's
    pub left: f64,
    /// Its top edge, from the viewport's
    pub top: f64,
    /// Its width
    pub width: f64,
    /// Its height
    pub height: f64,
}

impl Bounds {
    /// Its right edge, from the viewport's left
    pub fn right(&self) -> f64 {
        self.left + self.width
    }
}

/// The browser's side
#[cfg(feature = "web")]
mod web {
    use super::Bounds;
    use dioxus::prelude::MountedData;
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::Closure;

    /// An element on the page
    pub type El = web_sys::Element;

    /// The window
    fn window() -> Option<web_sys::Window> {
        web_sys::window()
    }

    /// The element Dioxus mounted
    ///
    /// # Arguments
    ///
    /// * `data` - What its `onmounted` was given
    pub fn mounted(data: &MountedData) -> Option<El> {
        data.try_as_web_event()
    }

    /// The innermost element an event happened on
    ///
    /// # Arguments
    ///
    /// * `data` - The event's data
    pub fn target<T: WebEventExt>(data: &T) -> Option<El>
    where
        T::WebEvent: JsCast,
    {
        // every web event is an `Event`, whose target is an element for the ones the canvas hears
        let event: web_sys::Event = data.try_as_web_event()?.unchecked_into();
        event.target()?.dyn_into().ok()
    }

    /// The element itself or its nearest ancestor that a selector matches
    ///
    /// # Arguments
    ///
    /// * `el` - Where to start
    /// * `sel` - The selector
    pub fn closest(el: &El, sel: &str) -> Option<El> {
        el.closest(sel).ok().flatten()
    }

    /// The first element under one that a selector matches
    ///
    /// # Arguments
    ///
    /// * `el` - Where to look under
    /// * `sel` - The selector
    pub fn query(el: &El, sel: &str) -> Option<El> {
        el.query_selector(sel).ok().flatten()
    }

    /// How many elements under one a selector matches
    ///
    /// # Arguments
    ///
    /// * `el` - Where to look under
    /// * `sel` - The selector
    pub fn count(el: &El, sel: &str) -> usize {
        el.query_selector_all(sel)
            .map_or(0, |list| list.length() as usize)
    }

    /// Every element under one that a selector matches, in document order
    ///
    /// # Arguments
    ///
    /// * `el` - Where to look under
    /// * `sel` - The selector
    pub fn query_all(el: &El, sel: &str) -> Vec<El> {
        let Ok(list) = el.query_selector_all(sel) else {
            return Vec::new();
        };
        (0..list.length())
            .filter_map(|i| list.item(i))
            .filter_map(|node| node.dyn_into::<El>().ok())
            .collect()
    }

    /// An element's attribute
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    /// * `name` - The attribute's name
    pub fn attr(el: &El, name: &str) -> Option<String> {
        el.get_attribute(name)
    }

    /// Where an element is on the page now
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    pub fn bounds(el: &El) -> Bounds {
        let r = el.get_bounding_client_rect();
        Bounds {
            left: r.left(),
            top: r.top(),
            width: r.width(),
            height: r.height(),
        }
    }

    /// Sends a pointer's events to an element until it is let go, wherever it moves
    ///
    /// # Arguments
    ///
    /// * `el` - The element to send them to
    /// * `pointer` - The pointer's id
    pub fn capture(el: &El, pointer: i32) {
        let _ = el.set_pointer_capture(pointer);
    }

    /// Lets a captured pointer go
    ///
    /// # Arguments
    ///
    /// * `el` - The element holding it
    /// * `pointer` - The pointer's id
    pub fn release(el: &El, pointer: i32) {
        let _ = el.release_pointer_capture(pointer);
    }

    /// Scrolls an element back to its top
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    pub fn scroll_to_top(el: &El) {
        el.set_scroll_top(0);
    }

    /// Gives an element the keyboard's focus
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    pub fn focus(el: &El) {
        if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
            let _ = el.focus();
        }
    }

    /// Shows an element full screen, or leaves full screen if anything is
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    pub fn toggle_fullscreen(el: &El) {
        let Some(doc) = window().and_then(|w| w.document()) else {
            return;
        };
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else {
            let _ = el.request_fullscreen();
        }
    }

    /// Whether an element is the one shown full screen
    ///
    /// # Arguments
    ///
    /// * `el` - The element
    pub fn is_fullscreen(el: &El) -> bool {
        window()
            .and_then(|w| w.document())
            .and_then(|d| d.fullscreen_element())
            .is_some_and(|e| e == *el)
    }

    /// The page's local storage, if it has any
    fn storage() -> Option<web_sys::Storage> {
        window()?.local_storage().ok().flatten()
    }

    /// What local storage keeps under a key
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    pub fn load(key: &str) -> Option<String> {
        storage()?.get_item(key).ok().flatten()
    }

    /// Keeps a value in local storage
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    /// * `value` - What to keep
    pub fn store(key: &str, value: &str) {
        if let Some(s) = storage() {
            let _ = s.set_item(key, value);
        }
    }

    /// Forgets what local storage keeps under a key
    ///
    /// # Arguments
    ///
    /// * `key` - The key
    pub fn forget(key: &str) {
        if let Some(s) = storage() {
            let _ = s.remove_item(key);
        }
    }

    /// Runs a closure once, after a delay, returning the timer to cancel it with
    ///
    /// # Arguments
    ///
    /// * `ms` - The delay
    /// * `f` - What to run
    pub fn later(ms: i32, f: impl FnOnce() + 'static) -> Option<i32> {
        // the browser frees the closure once it has run
        let cb = Closure::once_into_js(f);
        window()?
            .set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), ms)
            .ok()
    }

    /// Cancels a timer [`later`] started, if it hasn't run
    ///
    /// # Arguments
    ///
    /// * `timer` - The timer
    pub fn cancel(timer: i32) {
        if let Some(w) = window() {
            w.clear_timeout_with_handle(timer);
        }
    }

    /// How wide the window is, which reef.css's media queries go by
    pub fn viewport_width() -> f64 {
        window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|w| w.as_f64())
            .unwrap_or(0.0)
    }

    /// Milliseconds since the page started, for timing a layout
    pub fn now() -> f64 {
        window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now())
    }

    /// Warns in the console
    ///
    /// # Arguments
    ///
    /// * `msg` - What to say
    pub fn warn(msg: &str) {
        web_sys::console::warn_1(&msg.into());
    }

    /// A listener on the document, taken off again when this is dropped
    pub struct DocListener {
        /// The event it hears
        kind: &'static str,
        /// What it calls, kept while it listens
        callback: Closure<dyn FnMut()>,
    }

    impl Drop for DocListener {
        /// Takes the listener off the document
        fn drop(&mut self) {
            if let Some(doc) = window().and_then(|w| w.document()) {
                let _ = doc.remove_event_listener_with_callback(
                    self.kind,
                    self.callback.as_ref().unchecked_ref(),
                );
            }
        }
    }

    /// Listens on the document until the returned listener is dropped
    ///
    /// # Arguments
    ///
    /// * `kind` - The event's name
    /// * `f` - What to do each time it fires
    pub fn on_document(kind: &'static str, f: impl FnMut() + 'static) -> Option<DocListener> {
        let doc = window()?.document()?;
        let callback = Closure::<dyn FnMut()>::new(f);
        doc.add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())
            .ok()?;
        Some(DocListener { kind, callback })
    }

    /// Measures labels in the font an element is styled with
    pub struct Measure(reef_draw::web::Measure);

    impl Measure {
        /// Measures in an element's font
        ///
        /// # Arguments
        ///
        /// * `el` - The element, whose computed font family the labels inherit
        pub fn new(el: &El) -> Measure {
            Measure(reef_draw::web::Measure::new(el))
        }

        /// How wide a string is drawn, in pixels
        ///
        /// # Arguments
        ///
        /// * `s` - The text
        pub fn width(&self, s: &str) -> f64 {
            self.0.width(s)
        }
    }

    /// Reads the part of a doc a link points at, from its rendered page
    ///
    /// # Arguments
    ///
    /// * `href` - The link
    /// * `part` - What part of its section to show
    pub async fn read_doc(
        href: String,
        part: reef_draw::web::docs::Part,
    ) -> Result<String, String> {
        reef_draw::web::docs::read(&href, part)
            .await
            .map_err(|e| format!("{e:?}"))
    }

    /// Forgets every doc section read, so each is fetched again when it is next shown
    pub fn forget_docs() {
        reef_draw::web::docs::forget();
    }

    /// Writes SVG into a chart's overlay, the group drawn over the rest of it
    ///
    /// # Arguments
    ///
    /// * `chart` - What holds the chart's SVG
    /// * `svg` - The overlay's contents, or nothing to clear it
    pub fn overlay(chart: &El, svg: &str) {
        if let Some(o) = query(chart, ".reef-perf-overlay") {
            o.set_inner_html(svg);
        }
    }

    /// Lights one metric's table row and bars in the section around an element, and nothing
    /// else in the view
    ///
    /// # Arguments
    ///
    /// * `view` - The view's element, where every lit element is let go first
    /// * `el` - An element in the section
    /// * `metric` - The metric's index among its node's, or `None` to light nothing
    pub fn light(view: &El, el: &El, metric: Option<usize>) {
        // let go of whatever was lit
        if let Ok(list) = view.query_selector_all(".reef-perf-hl") {
            for k in 0..list.length() {
                if let Some(e) = list.item(k).and_then(|n| n.dyn_into::<El>().ok()) {
                    let _ = e.class_list().remove_1("reef-perf-hl");
                }
            }
        }
        // light the metric's row and bars in this section alone
        let (Some(k), Some(sec)) = (metric, closest(el, ".reef-perf, .reef-acc-item")) else {
            return;
        };
        if let Ok(list) = sec.query_selector_all(&format!(r#"[data-metric="{k}"]"#)) {
            for j in 0..list.length() {
                if let Some(e) = list.item(j).and_then(|n| n.dyn_into::<El>().ok()) {
                    let _ = e.class_list().add_1("reef-perf-hl");
                }
            }
        }
    }
}

/// The server's side: nothing is drawn there, so nothing is measured, stored or listened to
#[cfg(not(feature = "web"))]
mod server {
    use super::Bounds;
    use dioxus::prelude::MountedData;

    /// An element on the page, which the server never has
    #[derive(Debug, Clone, PartialEq)]
    pub struct El;

    /// No element is mounted on the server
    pub fn mounted(_data: &MountedData) -> Option<El> {
        None
    }

    /// No event happens on the server
    pub fn target<T>(_data: &T) -> Option<El> {
        None
    }

    /// No element has ancestors on the server
    pub fn closest(_el: &El, _sel: &str) -> Option<El> {
        None
    }

    /// No element has children on the server
    pub fn query(_el: &El, _sel: &str) -> Option<El> {
        None
    }

    /// No element has children on the server
    pub fn count(_el: &El, _sel: &str) -> usize {
        0
    }

    /// No element has children on the server
    pub fn query_all(_el: &El, _sel: &str) -> Vec<El> {
        Vec::new()
    }

    /// No element has attributes on the server
    pub fn attr(_el: &El, _name: &str) -> Option<String> {
        None
    }

    /// No element has a place on the server
    pub fn bounds(_el: &El) -> Bounds {
        Bounds::default()
    }

    /// No pointer is captured on the server
    pub fn capture(_el: &El, _pointer: i32) {}

    /// No pointer is captured on the server
    pub fn release(_el: &El, _pointer: i32) {}

    /// Nothing scrolls on the server
    pub fn scroll_to_top(_el: &El) {}

    /// Nothing has focus on the server
    pub fn focus(_el: &El) {}

    /// Nothing is full screen on the server
    pub fn toggle_fullscreen(_el: &El) {}

    /// Nothing is full screen on the server
    pub fn is_fullscreen(_el: &El) -> bool {
        false
    }

    /// The server keeps nothing
    pub fn load(_key: &str) -> Option<String> {
        None
    }

    /// The server keeps nothing
    pub fn store(_key: &str, _value: &str) {}

    /// The server keeps nothing
    pub fn forget(_key: &str) {}

    /// No timer runs on the server
    pub fn later(_ms: i32, _f: impl FnOnce() + 'static) -> Option<i32> {
        None
    }

    /// No timer runs on the server
    pub fn cancel(_timer: i32) {}

    /// The server has no window
    pub fn viewport_width() -> f64 {
        0.0
    }

    /// Nothing is timed on the server
    pub fn now() -> f64 {
        0.0
    }

    /// The server has no console to warn in
    pub fn warn(_msg: &str) {}

    /// The server listens to no document
    pub struct DocListener;

    /// The server listens to no document
    pub fn on_document(_kind: &'static str, _f: impl FnMut() + 'static) -> Option<DocListener> {
        None
    }

    /// Measures labels by their length, as reef-view does with no canvas
    pub struct Measure;

    impl Measure {
        /// A measure with no font
        pub fn new(_el: &El) -> Measure {
            Measure
        }

        /// About how wide a string is, from its length
        ///
        /// # Arguments
        ///
        /// * `s` - The text
        pub fn width(&self, s: &str) -> f64 {
            s.chars().count() as f64 * reef_draw::markup::FONT_PX * 0.55
        }
    }

    /// The part of a doc a sidebar shows, which the server never reads
    #[derive(Debug, Clone, PartialEq)]
    pub enum Part {
        /// All of the section
        Whole,
        /// A subsection
        Sub(String),
        /// The section's tables
        Tables,
    }

    /// No doc is read on the server
    pub async fn read_doc(_href: String, _part: Part) -> Result<String, String> {
        Err("docs are read in the browser".into())
    }

    /// No doc is kept on the server
    pub fn forget_docs() {}

    /// No chart is drawn on the server
    pub fn overlay(_chart: &El, _svg: &str) {}

    /// No chart is drawn on the server
    pub fn light(_view: &El, _el: &El, _metric: Option<usize>) {}
}

/// The part of a doc a sidebar shows: reef-draw's in the browser, a stand-in on the server
#[cfg(feature = "web")]
pub use reef_draw::web::docs::Part;
