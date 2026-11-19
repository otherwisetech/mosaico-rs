//! Formatting helpers with no UI.

/// 1234567 → "1 234 567" (narrow no-break space).
pub fn group(n: impl Into<u64>) -> String {
    let s = n.into().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out
}

/// Bytes as "834 KB", "1.2 GB" (decimal units; comma decimals in es/pt/fr).
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"]; // i18n-ok: unit symbols
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000. && u < UNITS.len() - 1 {
        v /= 1000.;
        u += 1;
    }
    let s = if u == 0 || v >= 100. { format!("{v:.0}") } else { format!("{v:.1}") };
    let s = if i18n::locale() == "en" { s } else { s.replace('.', ",") };
    format!("{s} {}", UNITS[u])
}

/// Localized month name, 1 = January.
pub fn month_name(m: u32) -> String {
    i18n::t!(&format!("common.month.{m}"))
}

pub fn month_names() -> [String; 12] {
    std::array::from_fn(|i| month_name(i as u32 + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(group(60769u64), "60\u{202F}769");
        assert_eq!(group(12u64), "12");
        i18n::set_locale("en");
        assert_eq!(human(999), "999 B");
        assert_eq!(human(1_234_567), "1.2 MB");
        assert_eq!(human(345_000_000_000), "345 GB");
        assert_eq!(month_name(5), "May");
        i18n::set_locale("es");
        assert_eq!(human(1_234_567), "1,2 MB");
        assert_eq!(month_names()[0], "enero");
    }
}
