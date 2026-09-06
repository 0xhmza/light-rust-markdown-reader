//! Cheap routing decisions; Windows supplies Unicode properties and shaping.
pub fn word_char(c: char) -> bool {
    if c.is_alphanumeric() {
        return true;
    }
    if c.is_ascii() {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        let mut units = [0; 2];
        let units = c.encode_utf16(&mut units);
        let mut properties = [0; 2];
        unsafe {
            crate::win32::GetStringTypeW(
                4,
                units.as_ptr(),
                units.len() as i32,
                properties.as_mut_ptr(),
            );
        }
        properties.iter().any(|p| p & 7 != 0)
    }
    #[cfg(not(target_os = "windows"))]
    matches!(c as u32, 0x300..=0x36f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f)
}

/// True when Windows must shape this character rather than map it to a glyph:
/// complex scripts, combining marks and bidirectional controls.
#[cfg(target_os = "windows")]
pub fn shapes(c: char) -> bool {
    !matches!(c as u32,
        0..=0x2ff | 0x370..=0x482 | 0x48a..=0x58f | 0x1e00..=0x1fff |
        0x2000..=0x200b | 0x2010..=0x2029 | 0x2030..=0x2065 |
        0x2070..=0x20cf | 0x2100..=0x2fff | 0x3000..=0x3029 |
        0x3030..=0x3098 | 0x309b..=0xa4cf | 0xac00..=0xd7a3 |
        0xf900..=0xfaff | 0xff00..=0xff9d | 0xffa0..=0xffef |
        0x1f000..=0x1faff | 0x20000..=0x323af)
}

#[cfg(target_os = "windows")]
pub fn right_to_left(text: &[u16]) -> bool {
    // Bounded stack buffer and first strong character, ignoring list markers,
    // digits and punctuation. Full bidi ordering remains DirectWrite's job.
    for chunk in text.chunks(128) {
        let mut properties = [0; 128];
        unsafe {
            crate::win32::GetStringTypeW(
                2,
                chunk.as_ptr(),
                chunk.len() as i32,
                properties.as_mut_ptr(),
            );
        }
        if let Some(p) = properties[..chunk.len()]
            .iter()
            .find(|&&p| p == 1 || p == 2)
        {
            return *p == 2;
        }
    }
    false
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn routing_and_first_strong_direction() {
        for text in [
            "English",
            "Grüße",
            "Ελληνικά",
            "Українська",
            "日本語",
            "中文",
            "한국어",
            "😀",
        ] {
            assert!(!text.chars().any(shapes), "{text}");
        }
        for text in [
            "العربية",
            "עברית",
            "हिन्दी",
            "বাংলা",
            "தமிழ்",
            "ไทย",
            "ខ្មែរ",
            "မြန်မာ",
            "cafe\u{301}",
            "か\u{3099}",
            "한",
            "a\u{2067}abc\u{2069}",
        ] {
            assert!(text.chars().any(shapes), "{text}");
        }
        for (text, rtl) in [
            ("123. العربية English", true),
            ("• עברית", true),
            ("English العربية", false),
            ("123...", false),
        ] {
            assert_eq!(right_to_left(&text.encode_utf16().collect::<Vec<_>>()), rtl);
        }
    }
}
