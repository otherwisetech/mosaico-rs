//! Small styled building blocks shared by the top bar, panels and screens.
use crate::theme::Tokens;
use gpui::*;
use gpui_component::IconName;
use otherwise_ui::tokens::icon as icon_size;
pub use otherwise_ui::{filled_button, icon, outline_button, section_title};

/// The family's square icon-only button with one of gpui-component's icons.
pub fn icon_button(t: &Tokens, id: impl Into<ElementId>, name: IconName, enabled: bool) -> Stateful<Div> {
    otherwise_ui::icon_button(t, id, enabled).child(icon(name, icon_size::TOOLBAR, if enabled { t.ink } else { t.ink_faint }))
}

/// A small rounded label.
pub fn badge(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> Div {
    div().flex_none().px(px(6.)).py(px(1.)).rounded(px(6.)).bg(bg).text_color(fg).text_size(px(11.)).font_weight(FontWeight::MEDIUM).child(text.into())
}

/// Thin horizontal progress bar, `frac` in 0..=1.
pub fn progress_bar(t: &Tokens, frac: f32, color: Hsla) -> Div {
    let frac = otherwise_ui::clamp_fraction(frac);
    div().w_full().h(px(otherwise_ui::tokens::size::PROGRESS)).rounded_full().bg(t.border).overflow_hidden().child(div().h_full().rounded_full().bg(color).w(relative(frac)))
}
