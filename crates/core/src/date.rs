//! Capture dates: where they came from, EXIF text parsing and file-name patterns.
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

/// Where an item's capture date came from, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DateSource {
    /// EXIF `DateTimeOriginal`, `CreateDate` (DateTimeDigitized) or `DateTime`.
    Exif,
    /// QuickTime / MP4 creation time.
    Video,
    /// A date written in the file name (`IMG_20230514_…`, `WhatsApp Image 2023-05-14 …`).
    Filename,
    /// The file's modification time: often the copy date, so the date is uncertain.
    Modified,
}

impl DateSource {
    pub const ALL: [DateSource; 4] = [DateSource::Exif, DateSource::Video, DateSource::Filename, DateSource::Modified];

    pub fn uncertain(self) -> bool {
        self == DateSource::Modified
    }
}

const MIN_YEAR: i32 = 1980;

fn plausible(d: NaiveDateTime) -> bool {
    let max = chrono::Local::now().year() + 1;
    (MIN_YEAR..=max).contains(&d.year())
}

/// `2023:05:14 10:22:33` (EXIF), also with `-` separators or a `T`; trailing sub-seconds or a
/// zone offset are ignored. All-zero dates (`0000:00:00 00:00:00`) are rejected.
pub fn parse_exif(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = if b.len() >= 19 { (num(11..13)?, num(14..16)?, num(17..19)?) } else { (0, 0, 0) };
    let dt = NaiveDate::from_ymd_opt(y as i32, mo, d)?.and_time(NaiveTime::from_hms_opt(h, mi, se.min(59))?);
    plausible(dt).then_some(dt)
}

/// A date (and, when present, a time) embedded in a file name. Recognises camera, phone and
/// messenger conventions: `IMG_20230514_102233`, `PXL_20230514_102233123`, `20230514_102233`,
/// `WhatsApp Image 2023-05-14 at 10.22.33`, `Screenshot 2023-05-14 at 10.22.33 PM`,
/// `VID-20230514-WA0001`, `2023-05-14 10.22.33`.
pub fn from_filename(name: &str) -> Option<NaiveDateTime> {
    let b = name.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    for start in 0..b.len() {
        if !digit(start) || (start > 0 && digit(start - 1)) {
            continue;
        }
        if let Some(dt) = date_at(b, start) {
            return Some(dt);
        }
    }
    None
}

fn date_at(b: &[u8], i: usize) -> Option<NaiveDateTime> {
    let digits = |at: usize, n: usize| -> Option<u32> {
        let s = b.get(at..at + n)?;
        s.iter().all(u8::is_ascii_digit).then(|| s.iter().fold(0, |a, d| a * 10 + (d - b'0') as u32))
    };
    let year = digits(i, 4)?;
    let mut p = i + 4;
    let sep = b.get(p).copied().filter(|c| matches!(c, b'-' | b'_' | b'.'));
    if sep.is_some() {
        p += 1;
    }
    let month = digits(p, 2)?;
    p += 2;
    if let Some(s) = sep {
        if b.get(p) != Some(&s) {
            return None;
        }
        p += 1;
    }
    let day = digits(p, 2)?;
    p += 2;
    let date = NaiveDate::from_ymd_opt(year as i32, month, day)?;
    // compact dates may not run into more digits (`202305141` is not a date), except a time
    if sep.is_none() && b.get(p).is_some_and(u8::is_ascii_digit) && digits(p, 6).is_none() {
        return None;
    }
    let time = time_at(b, p).unwrap_or(NaiveTime::MIN);
    let dt = date.and_time(time);
    plausible(dt).then_some(dt)
}

fn time_at(b: &[u8], mut p: usize) -> Option<NaiveTime> {
    let digits = |at: usize| -> Option<u32> {
        let s = b.get(at..at + 2)?;
        s.iter().all(u8::is_ascii_digit).then(|| ((s[0] - b'0') * 10 + (s[1] - b'0')) as u32)
    };
    // separator between date and time: "_", "-", "T", " ", " at " (and its es/pt/fr/de
    // screenshot forms); or none (compact 14 digits)
    let rest = &b[p..];
    if let Some(w) = [&b" at "[..], b" a la(s) ", b" a las ", " \u{e0} ".as_bytes(), " \u{e0}s ".as_bytes(), b" um "].iter().find(|w| rest.starts_with(w)) {
        p += w.len();
    } else if b.get(p).is_some_and(|c| matches!(c, b'_' | b'-' | b'T' | b' ')) {
        p += 1;
    }
    let h = digits(p)?;
    p += 2;
    let sep = b.get(p).copied().filter(|c| matches!(c, b'.' | b'-' | b':' | b'_'));
    if sep.is_some() {
        p += 1;
    }
    let m = digits(p)?;
    p += 2;
    if let Some(s) = sep {
        if b.get(p) != Some(&s) {
            return None;
        }
        p += 1;
    }
    let s = digits(p)?;
    p += 2;
    let rest = &b[p..];
    let pm = [&b" PM"[..], b"\xE2\x80\xAFPM", b" p.m.", b" p. m.", b"\xC2\xA0p.\xC2\xA0m."].iter().any(|w| rest.starts_with(w));
    let am = [&b" AM"[..], b"\xE2\x80\xAFAM", b" a.m.", b" a. m.", b"\xC2\xA0a.\xC2\xA0m."].iter().any(|w| rest.starts_with(w));
    let h = if pm {
        if h < 12 { h + 12 } else { h }
    } else if am && h == 12 {
        0
    } else {
        h
    };
    NaiveTime::from_hms_opt(h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap()
    }

    #[test]
    fn exif_text() {
        assert_eq!(parse_exif("2023:05:14 10:22:33"), Some(dt(2023, 5, 14, 10, 22, 33)));
        assert_eq!(parse_exif("2023:05:14 10:22:33\0"), Some(dt(2023, 5, 14, 10, 22, 33)));
        assert_eq!(parse_exif("2023-05-14T10:22:33-05:00"), Some(dt(2023, 5, 14, 10, 22, 33)));
        assert_eq!(parse_exif("0000:00:00 00:00:00"), None);
        assert_eq!(parse_exif("    :  :     :  :  "), None);
        assert_eq!(parse_exif("1970:01:01 00:00:00"), None);
    }

    #[test]
    fn filename_patterns() {
        let cases = [
            ("IMG_20230514_102233.jpg", dt(2023, 5, 14, 10, 22, 33)),
            ("PXL_20230514_102233123.jpg", dt(2023, 5, 14, 10, 22, 33)),
            ("20230514_102233.mp4", dt(2023, 5, 14, 10, 22, 33)),
            ("WhatsApp Image 2023-05-14 at 10.22.33.jpeg", dt(2023, 5, 14, 10, 22, 33)),
            ("WhatsApp Image 2023-05-14 at 10.22.33 (1).jpeg", dt(2023, 5, 14, 10, 22, 33)),
            ("Screenshot 2023-05-14 at 10.22.33.png", dt(2023, 5, 14, 10, 22, 33)),
            ("Screen Shot 2019-02-03 at 1.02.03 PM.png", dt(2019, 2, 3, 0, 0, 0)),
            ("Screenshot 2023-05-14 at 01.02.03 PM.png", dt(2023, 5, 14, 13, 2, 3)),
            ("VID-20230514-WA0001.mp4", dt(2023, 5, 14, 0, 0, 0)),
            ("IMG-20230514-WA0007.jpg", dt(2023, 5, 14, 0, 0, 0)),
            ("Photo 2023-05-14 10.22.33.jpg", dt(2023, 5, 14, 10, 22, 33)),
            ("20230514102233.jpg", dt(2023, 5, 14, 10, 22, 33)),
            ("signal-2023-05-14-102233.jpg", dt(2023, 5, 14, 10, 22, 33)),
            ("Captura de Pantalla 2023-07-03 a la(s) 11.48.11 p.m..jpg", dt(2023, 7, 3, 23, 48, 11)),
            ("Captura de Tela 2023-07-03 \u{e0}s 09.08.07.png", dt(2023, 7, 3, 9, 8, 7)),
            ("Capture d\u{2019}\u{e9}cran 2023-07-03 \u{e0} 09.08.07.png", dt(2023, 7, 3, 9, 8, 7)),
        ];
        for (name, want) in cases {
            assert_eq!(from_filename(name), Some(want), "{name}");
        }
        for name in ["IMG_1234.JPG", "DSC01234.ARW", "20231399_000000.jpg", "1234567890123.jpg", "IMG_2023051.jpg", "18990101_000000.jpg", "holiday.png"] {
            assert_eq!(from_filename(name), None, "{name}");
        }
    }
}
