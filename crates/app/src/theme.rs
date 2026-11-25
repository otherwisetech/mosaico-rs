//! The family's tokens come from otherwise-ui (github.com/otherwisetech/otherwise-ui, by tag):
//! Inter, the type scale, the warm neutrals and status colours (warn for uncertain dates and
//! renamed files), with Mosaico's terracotta accent (`otherwise_ui::accents::MOSAICO`, set in
//! main.rs). `Tokens` derefs to `otherwise_ui::Theme` (so `&Tokens` passes wherever the crate
//! wants `&Theme`) and adds the empty thumbnail tile.
use gpui::{App, Hsla, Window};
use otherwise_ui::theme::hex as c;
pub use otherwise_ui::tokens::SANS;

#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub ui: otherwise_ui::Theme,
    /// empty thumbnail tile
    pub tile: Hsla,
}

impl std::ops::Deref for Tokens {
    type Target = otherwise_ui::Theme;
    fn deref(&self) -> &otherwise_ui::Theme {
        &self.ui
    }
}

impl Tokens {
    pub fn new(ui: otherwise_ui::Theme) -> Self {
        Tokens { ui, tile: if ui.dark { c(0x262523) } else { c(0xE7E4DE) } }
    }
}

pub fn tokens(cx: &App) -> Tokens {
    Tokens::new(otherwise_ui::theme(cx))
}

/// Follows the system's light / dark appearance and sets the family font on gpui-component.
pub fn apply(window: Option<&mut Window>, cx: &mut App) {
    otherwise_ui::apply(otherwise_ui::Appearance::System, window, cx);
}
