//! Design tokens (same family as rama, diskmap and genealogy-rs) in mosaico's terracotta.
use gpui::{App, Hsla, rgb};
use gpui_component::ActiveTheme;

#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub ink: Hsla,
    pub ink_muted: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub danger: Hsla,
    /// uncertain dates, renamed files
    pub warn: Hsla,
    pub warn_soft: Hsla,
    pub ok: Hsla,
    /// empty thumbnail tile
    pub tile: Hsla,
    /// logo badge background (the icon's terracotta)
    pub brand: Hsla,
    pub dark: bool,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn a(hex: u32, alpha: f32) -> Hsla {
    let mut h = c(hex);
    h.a = alpha;
    h
}

impl Tokens {
    pub fn light() -> Self {
        Tokens {
            canvas: c(0xF2F1EE),
            surface: c(0xFFFFFF),
            border: c(0xE2DFD8),
            border_strong: c(0xC9C5BB),
            ink: c(0x1C1B19),
            ink_muted: c(0x6B675F),
            accent: c(0xB24E33),
            accent_soft: c(0xF8E9E3),
            danger: c(0xA3382A),
            warn: c(0x9A6408),
            warn_soft: c(0xFBF0D9),
            ok: c(0x2E7D4F),
            tile: c(0xE7E4DE),
            brand: c(0xC0583A),
            dark: false,
        }
    }

    pub fn dark() -> Self {
        Tokens {
            canvas: c(0x141413),
            surface: c(0x1C1B1A),
            border: c(0x302E2B),
            border_strong: c(0x4A4741),
            ink: c(0xF2F0EA),
            ink_muted: c(0xA19C93),
            accent: c(0xEB8C6B),
            accent_soft: a(0xD06A48, 0.22),
            danger: c(0xE0715F),
            warn: c(0xE7B556),
            warn_soft: a(0xE0A030, 0.18),
            ok: c(0x6FD39A),
            tile: c(0x262523),
            brand: c(0xB65438),
            dark: true,
        }
    }

    pub fn on_fill(&self) -> Hsla {
        if self.dark { c(0x141413) } else { c(0xFFFFFF) }
    }
}

pub fn tokens(cx: &App) -> Tokens {
    if cx.theme().mode.is_dark() { Tokens::dark() } else { Tokens::light() }
}

pub const SANS: &str = "Inter";
