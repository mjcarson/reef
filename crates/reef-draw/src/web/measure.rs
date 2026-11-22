//! How wide a box's text is, in the font the view draws it in

use crate::markup::FONT_PX;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, Element};

/// Measures text in the 13px font a view's labels are drawn in
pub struct Measure {
    /// A canvas context set to the labels' font, if the page could make one
    ctx: Option<CanvasRenderingContext2d>,
}

impl Measure {
    /// Measures in the font family a view's element is styled with
    ///
    /// # Arguments
    ///
    /// * `el` - The view's element, whose computed `font-family` the labels inherit
    pub fn new(el: &Element) -> Measure {
        Measure { ctx: context(el) }
    }

    /// How wide a string is drawn, in pixels
    ///
    /// # Arguments
    ///
    /// * `s` - The text
    pub fn width(&self, s: &str) -> f64 {
        // with no canvas, a rough width from the character count
        match &self.ctx {
            Some(c) => c
                .measure_text(s)
                .map(|m| m.width())
                .unwrap_or(s.len() as f64 * 7.0),
            None => s.chars().count() as f64 * FONT_PX * 0.55,
        }
    }
}

/// A canvas context set to the font the diagram's labels are drawn in
///
/// # Arguments
///
/// * `el` - The view's element
fn context(el: &Element) -> Option<CanvasRenderingContext2d> {
    let window = web_sys::window()?;
    let doc = window.document()?;
    // a canvas of its own, never put in the page
    let canvas: web_sys::HtmlCanvasElement = doc.create_element("canvas").ok()?.dyn_into().ok()?;
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    // the element's own font family, or the browser's sans-serif
    let family = window
        .get_computed_style(el)
        .ok()
        .flatten()
        .and_then(|s| s.get_property_value("font-family").ok())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "sans-serif".into());
    ctx.set_font(&format!("{FONT_PX}px {family}"));
    Some(ctx)
}
