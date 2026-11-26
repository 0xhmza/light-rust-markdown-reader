//! Clipboard helpers and Markdown span reconstruction for partial selections.
use crate::markdown::Style;
#[cfg(target_os = "windows")]
use crate::win32::*;

/// Copy UTF-8 `text` to the Windows clipboard as UTF-16 (`CF_UNICODETEXT`).
#[cfg(target_os = "windows")]
pub unsafe fn copy_text(hwnd: Hwnd, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    if OpenClipboard(hwnd) == 0 {
        return false;
    }
    EmptyClipboard();
    let mem = GlobalAlloc(0x0002, bytes); // GMEM_MOVEABLE
    if mem.is_null() {
        CloseClipboard();
        return false;
    }
    let dest = GlobalLock(mem) as *mut u16;
    if dest.is_null() {
        GlobalFree(mem);
        CloseClipboard();
        return false;
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), dest, wide.len());
    GlobalUnlock(mem);
    let ok = !SetClipboardData(13, mem).is_null(); // CF_UNICODETEXT = 13
    if !ok {
        GlobalFree(mem);
    }
    CloseClipboard();
    ok
}

/// Wrap a slice of plain text in its inline Markdown syntax (`**`, `*`, `` ` ``,
/// `~~`, `[text](href)`). Used when a selection covers part of a block.
pub fn format_markdown_span(text: &str, style: Style, href: Option<&str>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = if style.0 & Style::CODE != 0 {
        // Choose a backtick fence longer than any run of backticks inside `text`.
        let mut max_ticks = 0usize;
        let mut cur = 0usize;
        for b in text.bytes() {
            if b == b'`' {
                cur += 1;
                max_ticks = max_ticks.max(cur);
            } else {
                cur = 0;
            }
        }
        let fence = "`".repeat(max_ticks + 1);
        let pad = if text.starts_with('`') || text.ends_with('`') {
            " "
        } else {
            ""
        };
        format!("{fence}{pad}{text}{pad}{fence}")
    } else {
        text.to_owned()
    };
    if style.0 & Style::STRIKE != 0 {
        out = format!("~~{out}~~");
    }
    let bold = style.0 & Style::BOLD != 0;
    let italic = style.0 & Style::ITALIC != 0;
    match (bold, italic) {
        (true, true) => out = format!("***{out}***"),
        (true, false) => out = format!("**{out}**"),
        (false, true) => out = format!("*{out}*"),
        (false, false) => {}
    }
    if style.0 & Style::LINK != 0 {
        if let Some(url) = href {
            out = format!("[{out}]({url})");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_inline_markdown_spans_cleanly() {
        assert_eq!(format_markdown_span("hi", Style::default(), None), "hi");
        assert_eq!(
            format_markdown_span("hi", Style(Style::BOLD), None),
            "**hi**"
        );
        assert_eq!(
            format_markdown_span("hi", Style(Style::BOLD | Style::ITALIC), None),
            "***hi***"
        );
        assert_eq!(format_markdown_span("x", Style(Style::CODE), None), "`x`");
        assert_eq!(
            format_markdown_span("a`b", Style(Style::CODE), None),
            "``a`b``"
        );
        assert_eq!(
            format_markdown_span("doc", Style(Style::LINK), Some("./doc.md")),
            "[doc](./doc.md)"
        );
        assert_eq!(
            format_markdown_span("doc", Style(Style::LINK | Style::BOLD), Some("https://a")),
            "[**doc**](https://a)"
        );
    }

    #[test]
    fn handles_empty_nested_code_backticks_strike_and_link_without_href() {
        assert_eq!(format_markdown_span("", Style(Style::BOLD), None), "");
        assert_eq!(
            format_markdown_span("cut", Style(Style::STRIKE), None),
            "~~cut~~"
        );
        assert_eq!(
            format_markdown_span("em", Style(Style::ITALIC), None),
            "*em*"
        );
        // Leading/trailing backticks get single-space padding per Markdown spec.
        assert_eq!(
            format_markdown_span("`tick`", Style(Style::CODE), None),
            "`` `tick` ``"
        );
        assert_eq!(
            format_markdown_span("a``b", Style(Style::CODE), None),
            "```a``b```"
        );
        // Combined code + strike + bold + link.
        assert_eq!(
            format_markdown_span(
                "fn",
                Style(Style::CODE | Style::BOLD | Style::LINK),
                Some("./api.md")
            ),
            "[**`fn`**](./api.md)"
        );
        // LINK bit without an href degrades cleanly without broken `[x]()`.
        assert_eq!(
            format_markdown_span("plain", Style(Style::LINK), None),
            "plain"
        );
    }
}
