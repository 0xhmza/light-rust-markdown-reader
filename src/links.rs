//! Link resolution: intra-document `#anchor` jumps, local `.md` files (opened
//! in a new tab), and safe external URLs (`http://`, `https://`, `mailto:`).
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// Scroll to a heading in the current document.
    Anchor(String),
    /// Open a Markdown file in a new tab, optionally scrolling to `#anchor`.
    MarkdownFile {
        path: PathBuf,
        anchor: Option<String>,
    },
    /// Open an external URL in the system default handler.
    External(String),
}

const MD_EXTS: [&str; 4] = ["md", "markdown", "mdown", "mkd"];

/// Decode `%XX` percent-escapes (common in Markdown links with spaces like `my%20file.md`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Normalize `.` and `..` path segments lexically without hitting the filesystem.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(comp);
                }
            }
            other => out.push(other),
        }
    }
    out
}

pub fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| MD_EXTS.iter().any(|&known| ext.eq_ignore_ascii_case(known)))
}

/// Classify a clicked Markdown link target relative to `current_file`.
pub fn resolve(href: &str, current_file: Option<&Path>) -> Option<LinkAction> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if let Some(frag) = href.strip_prefix('#') {
        let slug = crate::markdown::heading_slug(&percent_decode(frag));
        return (!slug.is_empty()).then_some(LinkAction::Anchor(slug));
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return Some(LinkAction::External(href.to_owned()));
    }
    // Strip `file://` if present; reject other unknown/dangerous URI schemes.
    let raw_path = if let Some(rest) = href
        .strip_prefix("file:///")
        .or_else(|| href.strip_prefix("FILE:///"))
    {
        rest
    } else if has_uri_scheme(href) {
        return None;
    } else {
        href
    };
    let (path_part, anchor) = match raw_path.split_once('#') {
        Some((p, a)) => {
            let slug = crate::markdown::heading_slug(&percent_decode(a));
            (p, (!slug.is_empty()).then_some(slug))
        }
        None => (raw_path, None),
    };
    let path_part = path_part.split_once('?').map_or(path_part, |(p, _)| p);
    let decoded = percent_decode(path_part);
    if decoded.is_empty() {
        return anchor.map(LinkAction::Anchor);
    }
    let candidate = PathBuf::from(&decoded);
    if !is_markdown_path(&candidate) {
        return None;
    }
    let resolved = if candidate.is_absolute() {
        normalize_path(&candidate)
    } else if let Some(base) = current_file.and_then(Path::parent) {
        normalize_path(&base.join(&candidate))
    } else {
        normalize_path(&candidate)
    };
    Some(LinkAction::MarkdownFile {
        path: resolved,
        anchor,
    })
}

/// True for `scheme:...` where `scheme` is 2+ ASCII letters (so Windows drive
/// letters like `C:\foo.md` are NOT mistaken for a URI scheme).
fn has_uri_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_anchors_external_urls_and_markdown_paths() {
        let current = Path::new(r"C:\docs\guide\intro.md");

        assert_eq!(
            resolve("#Getting-Started", Some(current)),
            Some(LinkAction::Anchor("getting-started".to_owned()))
        );
        assert_eq!(
            resolve("https://example.com/a?b=1", Some(current)),
            Some(LinkAction::External("https://example.com/a?b=1".to_owned()))
        );
        assert_eq!(
            resolve("mailto:hi@example.com", Some(current)),
            Some(LinkAction::External("mailto:hi@example.com".to_owned()))
        );
        assert_eq!(
            resolve("../design.md#scrolling", Some(current)),
            Some(LinkAction::MarkdownFile {
                path: PathBuf::from(r"C:\docs\design.md"),
                anchor: Some("scrolling".to_owned()),
            })
        );
        assert_eq!(
            resolve("my%20notes.markdown", Some(current)),
            Some(LinkAction::MarkdownFile {
                path: PathBuf::from(r"C:\docs\guide\my notes.markdown"),
                anchor: None,
            })
        );
        assert_eq!(
            resolve(r"C:\other\readme.md", Some(current)),
            Some(LinkAction::MarkdownFile {
                path: PathBuf::from(r"C:\other\readme.md"),
                anchor: None,
            })
        );
        // Unsafe schemes and non-Markdown local executables are ignored.
        assert_eq!(resolve("javascript:alert(1)", Some(current)), None);
        assert_eq!(resolve("../run.exe", Some(current)), None);
    }

    #[test]
    fn handles_all_markdown_extensions_anchors_queries_and_security_rejections() {
        let current = Path::new(r"C:\workspace\docs\sub\page.md");

        // All 4 Markdown extensions (case-insensitive) plus query strings & anchors.
        for (href, expected_file, expected_anchor) in [
            ("./a.md", r"C:\workspace\docs\sub\a.md", None),
            ("b.MD?raw=1#Top", r"C:\workspace\docs\sub\b.MD", Some("top")),
            (
                "../c.markdown#Section%20Two",
                r"C:\workspace\docs\c.markdown",
                Some("section-two"),
            ),
            ("../../d.mdown", r"C:\workspace\d.mdown", None),
            (
                "notes.mkd#a-b",
                r"C:\workspace\docs\sub\notes.mkd",
                Some("a-b"),
            ),
            ("file:///C:/docs/ref.md#end", r"C:\docs\ref.md", Some("end")),
        ] {
            assert_eq!(
                resolve(href, Some(current)),
                Some(LinkAction::MarkdownFile {
                    path: PathBuf::from(expected_file),
                    anchor: expected_anchor.map(str::to_owned),
                }),
                "failed for {href}"
            );
        }

        // Uppercase safe external schemes still resolve.
        assert_eq!(
            resolve("HTTPS://Example.COM/Path", Some(current)),
            Some(LinkAction::External("HTTPS://Example.COM/Path".to_owned()))
        );
        assert_eq!(
            resolve("HTTP://localhost:8080/test", None),
            Some(LinkAction::External(
                "HTTP://localhost:8080/test".to_owned()
            ))
        );

        // Relative `.md` with no base file resolves lexically without panicking.
        assert_eq!(
            resolve("./guide/../readme.md", None),
            Some(LinkAction::MarkdownFile {
                path: PathBuf::from("readme.md"),
                anchor: None,
            })
        );

        // Dangerous schemes, empty links, and non-Markdown files must be rejected.
        for blocked in [
            "",
            "   ",
            "#",
            "###",
            "javascript:void(0)",
            "JAVASCRIPT:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///C:/Windows/System32/cmd.exe",
            "script.ps1",
            "payload.bat",
            "shortcut.lnk",
            "image.png",
        ] {
            assert_eq!(
                resolve(blocked, Some(current)),
                None,
                "should reject {blocked:?}"
            );
        }
    }
}
