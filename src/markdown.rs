//! A deliberately small, line-oriented Markdown reader, not a CommonMark engine.
//! All parsing is safe Rust. Markup we do not understand remains readable text.
//! Spans borrow directly from the source string (`Cow::Borrowed`) whenever no
//! escape/tab rewriting is needed, avoiding tens of thousands of heap allocations.
use std::borrow::Cow;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style(pub u8);
impl Style {
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 2;
    pub const CODE: u8 = 4;
    pub const LINK: u8 = 8;
    pub const STRIKE: u8 = 16;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span<'a> {
    pub text: Cow<'a, str>,
    pub style: Style,
    /// Link destination for `Style::LINK` spans, or `None` for non-links.
    pub href: Option<Cow<'a, str>>,
}

/// No paragraph of prose runs this long: about fourteen hundred words on one
/// unbroken line. Beyond it the text is machine-made, and shaping a single run
/// of that size costs time quadratic in its length, so it is broken up.
pub const LONGEST_BLOCK: usize = 8192;
/// Every block costs a row and a little heap, so a file of nothing but
/// newlines would otherwise be an out-of-memory abort. Past this the document
/// is cut, and the viewer says so rather than dying.
pub const LONGEST_DOCUMENT: usize = 1_000_000;
/// A table this wide is already unreadable, and the cell count is the product
/// of columns and rows, so it is where a malformed delimiter row would turn
/// into gigabytes.
pub const WIDEST_TABLE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    Quote,
    List(u8),
    Code,
    Rule,
    Gap,
    /// One table row. The payload marks the header row; text lives in `cells`.
    Row(bool),
}

/// A table cell: styled text plus its column alignment (0 left, 1 centre, 2 right).
#[derive(Debug)]
pub struct Cell<'a> {
    pub spans: Vec<Span<'a>>,
    pub align: u8,
}

#[cfg(test)]
#[derive(Debug)]
pub struct Block<'a> {
    pub kind: Kind,
    pub spans: Vec<Span<'a>>,
    pub cells: Vec<Cell<'a>>,
    /// Byte slice into the original Markdown source that produced this block.
    pub raw: Range<usize>,
}
#[cfg(test)]
impl<'a> Block<'a> {
    fn new(kind: Kind, spans: Vec<Span<'a>>, raw: Range<usize>) -> Self {
        Self {
            kind,
            spans,
            cells: vec![],
            raw,
        }
    }
}

/// Concatenate plain text of a slice of spans.
pub fn plain_text(spans: &[Span<'_>]) -> String {
    let cap: usize = spans.iter().map(|s| s.text.len()).sum();
    let mut out = String::with_capacity(cap);
    for span in spans {
        out.push_str(&span.text);
    }
    out
}

/// GitHub-style heading anchor slug for `#section` intra-document links.
pub fn heading_slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            dash = false;
        } else if c == '-' || c == '_' || c.is_whitespace() {
            if !out.is_empty() && !dash {
                out.push('-');
                dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Extract the clean link destination from inside `(...)`, stripping optional
/// `<...>` angle brackets and optional `"title"` / `'title'` suffix.
fn parse_link_target(raw: &str) -> &str {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            return rest[..end].trim();
        }
    }
    let url = if let Some(ws) = s.find(|c: char| c.is_whitespace()) {
        let after = s[ws..].trim_start();
        if after.starts_with('"') || after.starts_with('\'') || after.starts_with('(') {
            &s[..ws]
        } else {
            s
        }
    } else {
        s
    };
    url.trim()
}

fn push<'a>(spans: &mut Vec<Span<'a>>, text: &'a str, style: Style, href: Option<&'a str>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.style == style && last.href.as_deref() == href {
            last.text.to_mut().push_str(text);
            return;
        }
    }
    spans.push(Span {
        text: Cow::Borrowed(text),
        style,
        href: href.map(Cow::Borrowed),
    });
}

fn push_cow<'a>(
    spans: &mut Vec<Span<'a>>,
    text: Cow<'a, str>,
    style: Style,
    href: Option<Cow<'a, str>>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.style == style && last.href.as_deref() == href.as_deref() {
            last.text.to_mut().push_str(&text);
            return;
        }
    }
    spans.push(Span { text, style, href });
}

/// Fast lookup table for inline Markdown trigger bytes.
const SPECIAL: [bool; 256] = {
    let mut t = [false; 256];
    t[b'\\' as usize] = true;
    t[b'`' as usize] = true;
    t[b'[' as usize] = true;
    t[b'!' as usize] = true;
    t[b'*' as usize] = true;
    t[b'_' as usize] = true;
    t[b'~' as usize] = true;
    t
};

#[cfg(test)]
pub fn inline(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    parse_inline(text, Style::default(), None, 0, &mut spans);
    spans
}

fn parse_inline<'a>(
    text: &'a str,
    style: Style,
    href: Option<&'a str>,
    depth: u8,
    out: &mut Vec<Span<'a>>,
) {
    if depth >= 12 {
        push(out, text, style, href);
        return;
    }
    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() && !SPECIAL[bytes[pos] as usize] {
        pos += 1;
    }
    if pos == bytes.len() {
        push(out, text, style, href);
        return;
    }
    let mut plain = 0;
    let mut missing_delimiters = 0u8;
    let mut missing_code_runs = Vec::new();
    let mut links_exhausted = false;
    while pos < bytes.len() {
        while pos < bytes.len() && !SPECIAL[bytes[pos] as usize] {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let b = bytes[pos];
        let rest = &text[pos..];
        if b == b'\\' {
            if bytes.get(pos + 1).is_some_and(u8::is_ascii_punctuation) {
                push(out, &text[plain..pos], style, href);
                push(out, &rest[1..2], style, href);
                pos += 2;
                plain = pos;
                continue;
            }
            pos += 1;
            continue;
        }
        if b == b'`' {
            let n = rest.bytes().take_while(|&ch| ch == b'`').count();
            if !missing_code_runs.contains(&n) {
                if let Some(end) = closing_ticks(&rest[n..], n) {
                    push(out, &text[plain..pos], style, href);
                    push(out, &rest[n..n + end], Style(style.0 | Style::CODE), href);
                    pos += n + end + n;
                    plain = pos;
                    continue;
                }
                missing_code_runs.push(n);
            }
            pos += n;
            continue;
        }
        if b == b'[' || (b == b'!' && bytes.get(pos + 1) == Some(&b'[')) {
            if !links_exhausted {
                let image = b == b'!';
                let start = if image { 2 } else { 1 };
                let label_end = closing_bracket(rest, start);
                if let Some(label_end) = label_end.filter(|&end| rest[end..].starts_with("](")) {
                    let target = label_end + 2;
                    if let Some(end) = closing_paren(rest, target) {
                        push(out, &text[plain..pos], style, href);
                        let dest = parse_link_target(&rest[target..end]);
                        let link_href = (!dest.is_empty()).then_some(dest).or(href);
                        if image {
                            push(out, "[Image: ", style, href);
                        }
                        parse_inline(
                            &rest[start..label_end],
                            Style(style.0 | Style::LINK),
                            link_href,
                            depth + 1,
                            out,
                        );
                        if image {
                            push(out, "]", style, href);
                        }
                        pos += end + 1;
                        plain = pos;
                        continue;
                    }
                    links_exhausted = true;
                }
                links_exhausted |= label_end.is_none();
            }
            pos += 1;
            continue;
        }
        if matches!(b, b'*' | b'_' | b'~') {
            let candidates: &[(u8, &str, u8)] = match b {
                b'*' => &[
                    (0, "***", Style::BOLD | Style::ITALIC),
                    (2, "**", Style::BOLD),
                    (5, "*", Style::ITALIC),
                ],
                b'_' => &[
                    (1, "___", Style::BOLD | Style::ITALIC),
                    (3, "__", Style::BOLD),
                    (6, "_", Style::ITALIC),
                ],
                _ => &[(4, "~~", Style::STRIKE)],
            };
            let mut matched = false;
            for &(index, delim, flag) in candidates {
                if missing_delimiters & (1 << index) != 0 || !rest.starts_with(delim) {
                    continue;
                }
                if b == b'_'
                    && pos > 0
                    && text[..pos]
                        .chars()
                        .next_back()
                        .is_some_and(crate::text::word_char)
                {
                    continue;
                }
                let inner = &rest[delim.len()..];
                if inner.starts_with(char::is_whitespace) {
                    continue;
                }
                let Some(end) = inner.match_indices(delim).find_map(|(at, _)| {
                    let after = &inner[at + delim.len()..];
                    (at > 0
                        && !inner[..at].ends_with(char::is_whitespace)
                        && (b != b'_' || !after.chars().next().is_some_and(crate::text::word_char)))
                    .then_some(at)
                }) else {
                    missing_delimiters |= 1 << index;
                    continue;
                };
                if end > 0 && !inner[..end].ends_with(char::is_whitespace) {
                    push(out, &text[plain..pos], style, href);
                    parse_inline(&inner[..end], Style(style.0 | flag), href, depth + 1, out);
                    pos += delim.len() * 2 + end;
                    plain = pos;
                    matched = true;
                    break;
                }
            }
            if !matched {
                pos += 1;
            }
            continue;
        }
        pos += 1;
    }
    push(out, &text[plain..], style, href);
}

fn closing_ticks(text: &str, length: usize) -> Option<usize> {
    let mut pos = 0;
    while let Some(at) = text[pos..].find('`') {
        pos += at;
        let run = text[pos..].bytes().take_while(|&b| b == b'`').count();
        if run == length {
            return Some(pos);
        }
        pos += run;
    }
    None
}

fn closing_bracket(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' => depth += 1,
            ']' if depth == 0 => return Some(start + i),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn closing_paren(text: &str, start: usize) -> Option<usize> {
    let mut nesting = 0;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '(' => nesting += 1,
            ')' if nesting == 0 => return Some(start + i),
            ')' => nesting -= 1,
            _ => {}
        }
    }
    None
}

fn rule(line: &str) -> bool {
    let mut marks = line.chars().filter(|c| !c.is_whitespace());
    let Some(first @ ('-' | '*' | '_')) = marks.next() else {
        return false;
    };
    let mut count = 1;
    for c in marks {
        if c != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn fence(line: &str) -> Option<(u8, usize)> {
    let first = *line.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == first).count();
    (len >= 3).then_some((first, len))
}

fn split_cells_into<'a>(line: &'a str, out: &mut Vec<&'a str>) {
    out.clear();
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let (mut start, mut escaped) = (0, false);
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '|' {
            out.push(&line[start..i]);
            start = i + 1;
        }
    }
    out.push(&line[start..]);
}

fn alignments<'a>(line: &'a str, scratch: &mut Vec<&'a str>) -> Option<Vec<u8>> {
    split_cells_into(line, scratch);
    scratch
        .iter()
        .take(WIDEST_TABLE)
        .map(|cell| {
            let cell = cell.trim();
            let dashes = cell.trim_start_matches(':').trim_end_matches(':');
            (!dashes.is_empty() && dashes.bytes().all(|b| b == b'-')).then_some(
                match (cell.starts_with(':'), cell.ends_with(':')) {
                    (true, true) => 1,
                    (false, true) => 2,
                    _ => 0,
                },
            )
        })
        .collect()
}

/// Emit `spans` in chunks of at most `LONGEST_BLOCK` bytes, draining `spans`
/// while preserving its buffer capacity for reuse across lines.
fn emit_split<'a>(
    kind: Kind,
    spans: &mut Vec<Span<'a>>,
    raw: Range<usize>,
    count: &mut usize,
    on_block: &mut impl FnMut(Kind, &mut Vec<Span<'a>>, &mut Vec<Cell<'a>>, Range<usize>),
    empty_cells: &mut Vec<Cell<'a>>,
) {
    let total: usize = spans.iter().map(|s| s.text.len()).sum();
    if total <= LONGEST_BLOCK {
        if *count < LONGEST_DOCUMENT {
            *count += 1;
            on_block(kind, spans, empty_cells, raw);
        }
        spans.clear();
        return;
    }
    let mut chunk: Vec<Span<'a>> = Vec::new();
    let mut room = LONGEST_BLOCK;
    let drained = std::mem::take(spans);
    for span in drained {
        let style = span.style;
        let href = span.href;
        match span.text {
            Cow::Borrowed(mut text) => {
                while text.len() > room {
                    let mut cut = room;
                    while cut > 0 && !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    if let Some(space) = text[..cut].rfind(' ').filter(|at| *at * 8 > cut * 7) {
                        cut = space + 1;
                    }
                    if cut == 0 {
                        break;
                    }
                    push_cow(&mut chunk, Cow::Borrowed(&text[..cut]), style, href.clone());
                    if !chunk.is_empty() && *count < LONGEST_DOCUMENT {
                        *count += 1;
                        on_block(kind.clone(), &mut chunk, empty_cells, raw.clone());
                    }
                    chunk.clear();
                    room = LONGEST_BLOCK;
                    text = &text[cut..];
                }
                push_cow(&mut chunk, Cow::Borrowed(text), style, href);
                room -= text.len();
            }
            Cow::Owned(owned) => {
                let mut text = owned.as_str();
                while text.len() > room {
                    let mut cut = room;
                    while cut > 0 && !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    if let Some(space) = text[..cut].rfind(' ').filter(|at| *at * 8 > cut * 7) {
                        cut = space + 1;
                    }
                    if cut == 0 {
                        break;
                    }
                    push_cow(
                        &mut chunk,
                        Cow::Owned(text[..cut].to_owned()),
                        style,
                        href.clone(),
                    );
                    if !chunk.is_empty() && *count < LONGEST_DOCUMENT {
                        *count += 1;
                        on_block(kind.clone(), &mut chunk, empty_cells, raw.clone());
                    }
                    chunk.clear();
                    room = LONGEST_BLOCK;
                    text = &text[cut..];
                }
                push_cow(&mut chunk, Cow::Owned(text.to_owned()), style, href);
                room -= text.len();
            }
        }
    }
    if !chunk.is_empty() && *count < LONGEST_DOCUMENT {
        *count += 1;
        on_block(kind, &mut chunk, empty_cells, raw);
    }
    chunk.clear();
    *spans = chunk;
}

fn expand_tabs(line: &str) -> Cow<'_, str> {
    if line.as_bytes().contains(&b'\t') {
        Cow::Owned(line.replace('\t', "    "))
    } else {
        Cow::Borrowed(line)
    }
}

/// Cap one table cell's spans in place to `LONGEST_BLOCK` bytes without extra Vecs.
fn truncate_cell_spans(spans: &mut Vec<Span<'_>>) {
    let mut room = LONGEST_BLOCK;
    let mut keep = 0;
    for span in spans.iter_mut() {
        let len = span.text.len();
        if len <= room {
            room -= len;
            keep += 1;
            continue;
        }
        if room > 0 {
            let mut cut = room;
            while cut > 0 && !span.text.is_char_boundary(cut) {
                cut -= 1;
            }
            if cut > 0 {
                match &mut span.text {
                    Cow::Borrowed(s) => *s = &s[..cut],
                    Cow::Owned(s) => s.truncate(cut),
                }
                keep += 1;
            }
        }
        break;
    }
    spans.truncate(keep);
}

fn fill_table_row<'a>(
    line: &'a str,
    align: &[u8],
    raw_cells: &mut Vec<&'a str>,
    cells_buf: &mut Vec<Cell<'a>>,
) {
    split_cells_into(line, raw_cells);
    while cells_buf.len() < align.len() {
        cells_buf.push(Cell {
            spans: Vec::new(),
            align: 0,
        });
    }
    cells_buf.truncate(align.len());
    for (idx, &a) in align.iter().enumerate() {
        let text = raw_cells.get(idx).copied().unwrap_or("").trim();
        let cell = &mut cells_buf[idx];
        cell.align = a;
        cell.spans.clear();
        if !text.is_empty() {
            parse_inline(text, Style::default(), None, 0, &mut cell.spans);
            if cell.spans.iter().map(|s| s.text.len()).sum::<usize>() > LONGEST_BLOCK {
                truncate_cell_spans(&mut cell.spans);
            }
        }
    }
}

struct LineIter<'a> {
    src: &'a str,
    pos: usize,
}
impl<'a> LineIter<'a> {
    fn new(src: &'a str) -> Self {
        let pos = if src.starts_with('\u{feff}') { 3 } else { 0 };
        Self { src, pos }
    }
}
impl<'a> Iterator for LineIter<'a> {
    type Item = (&'a str, Range<usize>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.src.len() {
            return None;
        }
        let start = self.pos;
        let rest = &self.src[start..];
        let (line_end, next_pos) = match rest.as_bytes().iter().position(|&b| b == b'\n') {
            Some(rel) => (start + rel, start + rel + 1),
            None => (self.src.len(), self.src.len()),
        };
        let end = if line_end > start && self.src.as_bytes()[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        self.pos = next_pos;
        Some((&self.src[start..end], start..end))
    }
}

/// Stream parsed blocks directly to `on_block`, reusing internal span and cell
/// scratch buffers so parsing a document does O(1) temporary heap allocations.
/// Returns `true` if the document hit `LONGEST_DOCUMENT` and was truncated.
pub fn parse_each<'a>(
    source: &'a str,
    mut on_block: impl FnMut(Kind, &mut Vec<Span<'a>>, &mut Vec<Cell<'a>>, Range<usize>),
) -> bool {
    let mut spans_buf: Vec<Span<'a>> = Vec::with_capacity(16);
    let mut cells_buf: Vec<Cell<'a>> = Vec::new();
    let mut empty_cells: Vec<Cell<'a>> = Vec::new();
    let mut raw_cells: Vec<&'a str> = Vec::new();
    let mut count = 0usize;
    let mut last_was_gap = false;
    let mut fenced: Option<(u8, usize)> = None;
    let mut lines = LineIter::new(source).peekable();
    while let Some((raw, span_range)) = lines.next() {
        if count >= LONGEST_DOCUMENT {
            break;
        }
        let trimmed = raw.trim_start();
        if let Some((mark, length)) = fenced {
            if let Some((end_mark, end_length)) = fence(trimmed) {
                if mark == end_mark
                    && end_length >= length
                    && trimmed[end_length..].trim().is_empty()
                {
                    fenced = None;
                    continue;
                }
            }
            spans_buf.clear();
            spans_buf.push(Span {
                text: expand_tabs(raw),
                style: Style(Style::CODE),
                href: None,
            });
            last_was_gap = false;
            emit_split(
                Kind::Code,
                &mut spans_buf,
                span_range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
            continue;
        }
        if let Some(f) = fence(trimmed) {
            fenced = Some(f);
            continue;
        }
        if trimmed.is_empty() {
            if !last_was_gap {
                last_was_gap = true;
                spans_buf.clear();
                count += 1;
                on_block(Kind::Gap, &mut spans_buf, &mut empty_cells, span_range);
            }
            continue;
        }
        last_was_gap = false;
        if rule(trimmed) {
            spans_buf.clear();
            count += 1;
            on_block(Kind::Rule, &mut spans_buf, &mut empty_cells, span_range);
            continue;
        }
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes)
            && (trimmed.len() == hashes || trimmed.as_bytes()[hashes].is_ascii_whitespace())
        {
            let mut content = trimmed[hashes..].trim();
            let without_closing = content.trim_end_matches('#');
            if without_closing.ends_with(char::is_whitespace) {
                content = without_closing.trim_end();
            }
            spans_buf.clear();
            parse_inline(content, Style::default(), None, 0, &mut spans_buf);
            emit_split(
                Kind::Heading(hashes as u8),
                &mut spans_buf,
                span_range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
            continue;
        }
        if trimmed.as_bytes().contains(&b'|') {
            if let Some(align) = lines
                .peek()
                .map(|(next, _)| next.trim())
                .filter(|next| next.as_bytes().contains(&b'|') || next.as_bytes().contains(&b'-'))
                .and_then(|next| alignments(next, &mut raw_cells))
            {
                let delim_end = lines.next().map_or(span_range.end, |(_, r)| r.end);
                spans_buf.clear();
                fill_table_row(trimmed, &align, &mut raw_cells, &mut cells_buf);
                count += 1;
                on_block(
                    Kind::Row(true),
                    &mut spans_buf,
                    &mut cells_buf,
                    span_range.start..delim_end,
                );
                while count < LONGEST_DOCUMENT {
                    let Some(&(next, _)) = lines.peek() else {
                        break;
                    };
                    let next_trim = next.trim_start();
                    if next_trim.as_bytes().contains(&b'|') && fence(next_trim).is_none() {
                        let (_, r) = lines.next().unwrap();
                        spans_buf.clear();
                        fill_table_row(next_trim, &align, &mut raw_cells, &mut cells_buf);
                        count += 1;
                        on_block(Kind::Row(false), &mut spans_buf, &mut cells_buf, r);
                    } else {
                        break;
                    }
                }
                continue;
            }
        }
        if trimmed.starts_with('>') {
            spans_buf.clear();
            parse_inline(
                trimmed.trim_start_matches('>').trim_start(),
                Style::default(),
                None,
                0,
                &mut spans_buf,
            );
            emit_split(
                Kind::Quote,
                &mut spans_buf,
                span_range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
            continue;
        }
        let indent = raw.len() - trimmed.len();
        let bullet = ["- ", "* ", "+ "].iter().any(|p| trimmed.starts_with(p));
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        let ordered = digits > 0
            && digits <= 9
            && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "));
        if bullet || ordered {
            let offset = if bullet { 2 } else { digits + 2 };
            let content = &trimmed[offset..];
            let (marker, body): (Cow<'_, str>, &str) = if content.starts_with("[ ] ") {
                (Cow::Borrowed("[ ] "), &content[4..])
            } else if content.starts_with("[x] ") || content.starts_with("[X] ") {
                (Cow::Borrowed("[x] "), &content[4..])
            } else if ordered {
                (Cow::Owned(format!("{}. ", &trimmed[..digits])), content)
            } else {
                (Cow::Borrowed("• "), content)
            };
            spans_buf.clear();
            spans_buf.push(Span {
                text: marker,
                style: Style::default(),
                href: None,
            });
            parse_inline(body, Style::default(), None, 0, &mut spans_buf);
            emit_split(
                Kind::List((indent / 2).min(16) as u8),
                &mut spans_buf,
                span_range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
        } else if raw.starts_with("    ") || raw.starts_with('\t') {
            let content = raw
                .strip_prefix("    ")
                .or_else(|| raw.strip_prefix('\t'))
                .unwrap_or(raw);
            spans_buf.clear();
            spans_buf.push(Span {
                text: expand_tabs(content),
                style: Style(Style::CODE),
                href: None,
            });
            emit_split(
                Kind::Code,
                &mut spans_buf,
                span_range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
        } else {
            let mut kind = Kind::Paragraph;
            let mut range = span_range;
            if let Some(&(next_raw, ref next_range)) = lines.peek() {
                let next_trim = next_raw.trim_start().trim_end();
                if !next_trim.is_empty()
                    && (next_trim.bytes().all(|b| b == b'=')
                        || next_trim.bytes().all(|b| b == b'-'))
                {
                    kind = Kind::Heading(if next_trim.starts_with('=') { 1 } else { 2 });
                    range.end = next_range.end;
                    lines.next();
                }
            }
            spans_buf.clear();
            parse_inline(trimmed, Style::default(), None, 0, &mut spans_buf);
            emit_split(
                kind,
                &mut spans_buf,
                range,
                &mut count,
                &mut on_block,
                &mut empty_cells,
            );
        }
    }
    count >= LONGEST_DOCUMENT
}

#[cfg(test)]
pub fn parse(source: &str) -> Vec<Block<'_>> {
    let mut blocks: Vec<Block<'_>> = Vec::with_capacity((source.len() / 48).min(65_536));
    parse_each(source, |kind, spans, cells, raw| {
        let mut block = Block::new(kind, std::mem::take(spans), raw);
        if !cells.is_empty() {
            block.cells = cells
                .iter_mut()
                .map(|c| Cell {
                    spans: std::mem::take(&mut c.spans),
                    align: c.align,
                })
                .collect();
        }
        blocks.push(block);
    });
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plain(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }
    #[test]
    fn inline_styles_and_unicode() {
        let s = inline("Grüße **bold** *italics* `a*b` ~~gone~~ [世界](https://a/(b)) 😀");
        assert_eq!(plain(&s), "Grüße bold italics a*b gone 世界 😀");
        assert!(s
            .iter()
            .any(|s| s.text == "bold" && s.style.0 == Style::BOLD));
        assert!(s
            .iter()
            .any(|s| s.text == "a*b" && s.style.0 == Style::CODE));
        assert!(s.iter().any(|s| {
            s.text == "世界"
                && s.style.0 == Style::LINK
                && s.href.as_deref() == Some("https://a/(b)")
        }));
    }
    #[test]
    fn link_targets_preserve_paths_anchors_and_strip_titles() {
        let s = inline(
            "[doc](./docs/design.md#scrolling \"Design\") and [sp](<my file.md>) and [top](#intro)",
        );
        let links: Vec<(&str, Option<&str>)> = s
            .iter()
            .filter(|s| s.style.0 & Style::LINK != 0)
            .map(|s| (s.text.as_ref(), s.href.as_deref()))
            .collect();
        assert_eq!(
            links,
            [
                ("doc", Some("./docs/design.md#scrolling")),
                ("sp", Some("my file.md")),
                ("top", Some("#intro")),
            ]
        );
        assert_eq!(heading_slug("Hello, World! (2026)"), "hello-world-2026");
        assert_eq!(heading_slug("  C++ & Rust  "), "c-rust");
    }
    #[test]
    fn plain_spans_borrow_without_allocating() {
        let s = inline("Plain text with **bold** and `code` and [link](a.md)");
        assert!(s.iter().all(|span| matches!(span.text, Cow::Borrowed(_))));
    }
    #[test]
    fn nested_and_literal_markup() {
        assert_eq!(
            plain(&inline(
                "**bold *inner* text** \\*literal\\* snake_case unclosed**"
            )),
            "bold inner text *literal* snake_case unclosed**"
        );
        assert_eq!(inline("***both***")[0].style.0, Style::BOLD | Style::ITALIC);
        assert_eq!(plain(&inline("![alt](pic.png)")), "[Image: alt]");
    }
    #[test]
    fn blocks_and_fences() {
        let src = "\u{feff}# Title\r\n\r\n- [x] done\n> quote\n```rust\n**literal**\n````\n\n---\n";
        let b = parse(src);
        assert_eq!(b[0].kind, Kind::Heading(1));
        assert_eq!(&src[b[0].raw.clone()], "# Title");
        assert_eq!(b[2].kind, Kind::List(0));
        assert_eq!(&src[b[2].raw.clone()], "- [x] done");
        assert_eq!(b[4].kind, Kind::Code);
        assert_eq!(plain(&b[4].spans), "**literal**");
        assert_eq!(b.last().unwrap().kind, Kind::Rule);
    }
    #[test]
    fn setext_and_unclosed_fence() {
        let src = "Title\n===\n\n~~~~\ntext\n~~~\n";
        let b = parse(src);
        assert_eq!(b[0].kind, Kind::Heading(1));
        assert_eq!(&src[b[0].raw.clone()], "Title\n===");
        assert_eq!(b[3].kind, Kind::Code);
        assert_eq!(plain(&b[3].spans), "~~~");
    }
    #[test]
    fn tables_need_a_delimiter_row() {
        let blocks =
            parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 |\n\nnot \\| a table\n| pipes | only |\n");
        assert_eq!(blocks[0].kind, Kind::Row(true));
        assert_eq!(blocks[1].kind, Kind::Row(false));
        assert_eq!(plain(&blocks[0].cells[1].spans), "b");
        assert_eq!(plain(&blocks[1].cells[0].spans), "1");
        assert_eq!(
            blocks[0].cells.iter().map(|c| c.align).collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(blocks[2].kind, Kind::Row(false));
        assert_eq!(blocks[2].cells.len(), 2);
        assert_eq!(plain(&blocks[2].cells[1].spans), "");
        assert_eq!(blocks[4].kind, Kind::Paragraph);
        assert_eq!(plain(&blocks[4].spans), "not | a table");
        assert_eq!(blocks[5].kind, Kind::Paragraph);
        assert_eq!(plain(&blocks[5].spans), "| pipes | only |");
    }
    #[test]
    fn over_long_lines_and_over_wide_tables_are_bounded() {
        let words = "alpha beta gamma delta ".repeat(1_000);
        let blocks = parse(&words);
        assert!(blocks.len() > 1);
        assert_eq!(
            blocks.iter().map(|b| plain(&b.spans)).collect::<String>(),
            words
        );
        assert!(
            blocks[0].spans.last().unwrap().text.ends_with(' '),
            "the break belongs at a word boundary"
        );
        let solid = "x".repeat(LONGEST_BLOCK * 3);
        let blocks = parse(&solid);
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks.iter().map(|b| plain(&b.spans)).collect::<String>(),
            solid
        );
        let wide = "😀".repeat(LONGEST_BLOCK);
        assert_eq!(
            parse(&wide)
                .iter()
                .map(|b| plain(&b.spans))
                .collect::<String>(),
            wide
        );
        let table = format!("|{}\n|{}\n", "a|".repeat(5_000), "-|".repeat(5_000));
        let blocks = parse(&table);
        assert!(blocks.iter().all(|b| b.cells.len() <= WIDEST_TABLE));
        let huge = format!("| {} |\n|---|\n", "z".repeat(LONGEST_BLOCK * 2));
        for block in parse(&huge) {
            for cell in &block.cells {
                let length: usize = cell.spans.iter().map(|s| s.text.len()).sum();
                assert!(length <= LONGEST_BLOCK);
            }
        }
    }
    #[test]
    fn table_cells_keep_inline_markup() {
        let blocks = parse("| **b** | `c` |\n| --- | --- |\n| [x](u) | ~~y~~ |\n");
        assert_eq!(blocks[0].cells[0].spans[0].style.0, Style::BOLD);
        assert_eq!(blocks[0].cells[1].spans[0].style.0, Style::CODE);
        assert_eq!(blocks[1].cells[0].spans[0].style.0, Style::LINK);
        assert_eq!(blocks[1].cells[0].spans[0].href.as_deref(), Some("u"));
        assert_eq!(blocks[1].cells[1].spans[0].style.0, Style::STRIKE);
    }
    #[test]
    fn random_markup_never_panics_and_never_loses_text() {
        let alphabet: Vec<char> = "*_~`[]()!\\#>-|:0123456789. \tabc\u{e9}\u{5d0}\u{628}\u{939}\u{4e2d}\u{1f600}\u{301}\u{200d}\u{fe0f}\u{202e}\u{feff}"
            .chars()
            .collect();
        let mut seed = 0x2026_09_06u64;
        let mut random = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for case in 0..3000 {
            let length = random() % 200;
            let source: String = (0..length)
                .map(|_| alphabet[random() % alphabet.len()])
                .collect();
            let blocks = parse(&source);
            for block in &blocks {
                let length: usize = block.spans.iter().map(|s| s.text.len()).sum();
                assert!(length <= LONGEST_BLOCK, "case {case}: {source:?}");
                assert!(block.cells.len() <= WIDEST_TABLE);
            }
            let kept: String = blocks
                .iter()
                .flat_map(|block| {
                    block.spans.iter().map(|s| s.text.as_ref()).chain(
                        block
                            .cells
                            .iter()
                            .flat_map(|c| c.spans.iter().map(|s| s.text.as_ref())),
                    )
                })
                .collect();
            for c in kept.chars() {
                assert!(
                    source.contains(c) || c == ' ' || c == '\u{2022}',
                    "case {case}: {c:?} appeared from nowhere in {source:?}"
                );
            }
        }
    }
    #[test]
    fn empty_and_malformed_are_safe() {
        assert!(parse("").is_empty());
        for text in [
            "[",
            "![",
            "\\",
            "`",
            "***",
            "[a](",
            "é_世",
            "#",
            "\0",
            "|",
            "|-|",
            "|a|",
            "-",
            "|\n|-|\n|",
            "a|b\n:-:",
        ] {
            let _ = parse(text);
        }
        let large = "😀x".repeat(100_000);
        let blocks = parse(&large);
        assert!(blocks.len() > 1 && blocks.iter().all(|b| b.kind == Kind::Paragraph));
        assert_eq!(
            blocks.iter().map(|b| plain(&b.spans)).collect::<String>(),
            large
        );
        for block in &blocks {
            let length: usize = block.spans.iter().map(|s| s.text.len()).sum();
            assert!(length <= LONGEST_BLOCK, "{length} bytes in one block");
        }
        let brackets = "[".repeat(100_000);
        assert_eq!(plain(&inline(&brackets)), brackets);
        let destinations = "[a](".repeat(25_000);
        assert_eq!(plain(&inline(&destinations)), destinations);
    }
    #[test]
    fn code_delimiters_must_match_the_whole_run() {
        let spans = inline("`a `` b` and ``c ` d``");
        assert_eq!(plain(&spans), "a `` b and c ` d");
        assert_eq!(spans[0].style.0, Style::CODE);
        assert_eq!(plain(&inline("``unclosed```")), "``unclosed```");
    }
    #[test]
    fn literal_brackets_do_not_swallow_later_links() {
        let spans = inline("[note] then [link](url) and [a [nested] label](target)");
        assert_eq!(plain(&spans), "[note] then link and a [nested] label");
        assert_eq!(spans[0].style.0, 0);
        assert_eq!(spans[1].style.0, Style::LINK);
        assert_eq!(spans[1].href.as_deref(), Some("url"));
    }
    #[test]
    fn underscores_respect_unicode_word_boundaries() {
        for literal in [
            "_ok_bad",
            "foo _bar_baz",
            "cafe\u{301}_foo_",
            "中文_变量_",
            "हिन्दी_नाम_",
            "a_\u{301}b_",
        ] {
            let spans = inline(literal);
            assert_eq!(plain(&spans), literal);
            assert!(
                spans.iter().all(|s| s.style == Style::default()),
                "{literal}"
            );
        }
        assert_eq!(plain(&inline("_ok_bad_ and _fin_")), "ok_bad and fin");
        assert_eq!(inline("_العربية_")[0].style.0, Style::ITALIC);
    }
    #[test]
    fn heading_underlines_allow_trailing_spaces() {
        for (underline, level) in [("---  ", 2), ("===\t", 1)] {
            let src = format!("日本語\n{underline}\n");
            let blocks = parse(&src);
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].kind, Kind::Heading(level));
        }
    }
    #[test]
    fn languages_survive_all_inline_styles() {
        for text in [
            "Grüße",
            "Ελληνικά",
            "Українська",
            "日本語",
            "한국어",
            "العربية",
            "עברית",
            "हिन्दी",
            "தமிழ்",
            "ไทย",
            "Tiếng Việt",
        ] {
            let line = format!("**{text}** *{text}* `{text}` ~~{text}~~ [{text}](https://a)");
            let spans = inline(&line);
            assert_eq!(
                plain(&spans),
                format!("{text} {text} {text} {text} {text}"),
                "{text}"
            );
            for flag in [
                Style::BOLD,
                Style::ITALIC,
                Style::CODE,
                Style::STRIKE,
                Style::LINK,
            ] {
                assert!(
                    spans.iter().any(|s| s.text == text && s.style.0 == flag),
                    "{text} missing style {flag}"
                );
            }
        }
    }
    #[test]
    fn heading_slugs_handle_unicode_punctuation_and_separators() {
        assert_eq!(heading_slug("Hello, World!"), "hello-world");
        assert_eq!(
            heading_slug("  -- Leading & Trailing --  "),
            "leading-trailing"
        );
        assert_eq!(
            heading_slug("snake_case and kebab-case"),
            "snake-case-and-kebab-case"
        );
        assert_eq!(heading_slug("C++ / Rust (v2.0)"), "c-rust-v20");
        assert_eq!(heading_slug("日本語 見出し"), "日本語-見出し");
        assert_eq!(heading_slug("Grüße aus München"), "grüße-aus-münchen");
        assert_eq!(heading_slug("!!!"), "");
    }
    #[test]
    fn parser_edge_cases_escapes_crlf_tasks_and_code_tabs() {
        // Escaped markers stay literal and do not trigger styles or table splits.
        let escaped = inline(r"\*not italic\* \`not code\` \[not link\](x)");
        assert_eq!(plain(&escaped), "*not italic* `not code` [not link](x)");
        assert!(escaped.iter().all(|s| s.style == Style::default()));

        // Angle-bracket link with spaces and title stripping.
        let link = inline(r#"[Spec](<./my docs/spec v1.md> "Title")"#);
        assert_eq!(link.len(), 1);
        assert_eq!(link[0].style.0, Style::LINK);
        assert_eq!(link[0].href.as_deref(), Some("./my docs/spec v1.md"));

        // CRLF + BOM raw byte ranges map back to valid source slices without `\r`.
        let crlf = "\u{feff}# Title\r\n\r\n- [ ] Todo\r\n- [x] Done\r\n1) First\r\n";
        let blocks = parse(crlf);
        assert_eq!(blocks[0].kind, Kind::Heading(1));
        assert_eq!(&crlf[blocks[0].raw.clone()], "# Title");
        assert_eq!(blocks[1].kind, Kind::Gap);
        assert_eq!(plain(&blocks[2].spans), "[ ] Todo");
        assert_eq!(plain(&blocks[3].spans), "[x] Done");
        assert_eq!(plain(&blocks[4].spans), "1. First");

        // Code blocks without tabs borrow directly; with tabs expand cleanly.
        let code_src = "```\nlet x = 1;\n\tindented\n```\n";
        let code_blocks = parse(code_src);
        assert_eq!(code_blocks.len(), 2);
        assert!(matches!(
            code_blocks[0].spans[0].text,
            Cow::Borrowed("let x = 1;")
        ));
        assert!(matches!(code_blocks[1].spans[0].text, Cow::Owned(_)));
        assert_eq!(code_blocks[1].spans[0].text, "    indented");

        // Escaped pipe inside a table cell does not split into extra columns.
        let tbl = parse("| Expr | Result |\n|---|---|\n| a \\| b | ok |\n");
        assert_eq!(tbl.len(), 2);
        assert_eq!(tbl[1].cells.len(), 2);
        assert_eq!(plain(&tbl[1].cells[0].spans), "a | b");
        assert_eq!(plain(&tbl[1].cells[1].spans), "ok");
    }
    #[test]
    fn parse_each_reuses_buffers_and_matches_all_fixtures() {
        for fixture in [
            include_str!("../examples/showcase.md"),
            include_str!("../examples/tables.md"),
            include_str!("../examples/emoji.md"),
            include_str!("../examples/languages.md"),
            include_str!("../examples/complex-scripts.md"),
        ] {
            let expected = parse(fixture);
            let mut streamed = 0usize;
            let truncated = parse_each(fixture, |kind, spans, cells, raw| {
                let exp = &expected[streamed];
                assert_eq!(kind, exp.kind);
                assert_eq!(raw, exp.raw);
                assert_eq!(plain(spans), plain(&exp.spans));
                assert_eq!(cells.len(), exp.cells.len());
                for (c, ec) in cells.iter().zip(&exp.cells) {
                    assert_eq!(c.align, ec.align);
                    assert_eq!(plain(&c.spans), plain(&ec.spans));
                }
                streamed += 1;
            });
            assert!(!truncated);
            assert_eq!(streamed, expected.len());
        }
    }
    #[test]
    fn streaming_parser_handles_100k_lines_cleanly() {
        let mut big = String::with_capacity(6_500_000);
        for i in 0..100_000 {
            if i % 1000 == 0 {
                big.push_str("# Section\n");
            } else {
                big.push_str("Line with **bold**, `code`, and [link](./a.md) plain text.\n");
            }
        }
        let mut blocks = 0usize;
        let mut headings = 0usize;
        let truncated = parse_each(&big, |kind, spans, _, _| {
            blocks += 1;
            if matches!(kind, Kind::Heading(1)) {
                headings += 1;
            }
            assert!(!spans.is_empty());
        });
        assert!(!truncated);
        assert_eq!(blocks, 100_000);
        assert_eq!(headings, 100);
    }
    #[test]
    fn adversarial_parser_torture_emoji_scripts_and_pathological_syntax() {
        let zalgo = format!(
            "Z{}a{}l{}g{}o",
            "\u{0301}\u{0323}\u{0308}".repeat(12),
            "\u{0336}".repeat(10),
            "\u{0300}".repeat(10),
            "\u{0315}".repeat(10)
        );
        let torture = format!(
            concat!(
                "\u{feff}# 👨‍👩‍👧‍👦 👩‍🚀 🏳️‍🌈 🏴󠁧󠁢󠁳󠁣󠁴󠁿 👍🏽 🇲🇦🇯🇵🇺🇸 1️⃣#️⃣*️⃣ ❤︎❤️ \u{200d}\u{fe0f}\r\n\r\n",
                "## العربية العَرَبِيَّةُ — שָׁלוֹם — हिन्दी क्ष त्र ज्ञ — বাংলা — தமிழ் — ภาษาไทย — ខ្មែរ — မြန်မာ — བོད་སྐད — 日本語 𠀀 — 한국어 — Tiếng Việt\n\n",
                "Bidi controls: \u{202e}override\u{202c} \u{2066}isolate\u{2069} \u{200f}rlm\u{200e} {zalgo}\n\n",
                "**[👩‍👩‍👧‍👦 link](<./docs/my 日本語 file.md#section> \"title\")** and [](empty) and [no-url]() and [deep](https://a/(b/(c)))\n\n",
                "~~~~\n```\ninside tilde fence\n```\n~~~~\n\n",
                "| {} |\n| {} |\n| {} |\n\n",
                "{}\n{}\n{}\n"
            ),
            (0..80).map(|i| format!("col{i} 😀")).collect::<Vec<_>>().join(" | "),
            (0..80).map(|_| ":---:").collect::<Vec<_>>().join(" | "),
            (0..80).map(|i| format!("v{i} \\| العربية")).collect::<Vec<_>>().join(" | "),
            "*_~`".repeat(5_000),
            "[[[[".repeat(4_000),
            "👨‍👩‍👧‍👦".repeat(600),
            zalgo = zalgo,
        );
        let blocks = parse(&torture);
        assert!(!blocks.is_empty());
        assert!(matches!(blocks[0].kind, Kind::Heading(1)));
        // Every emitted block must remain within safety limits and valid UTF-8.
        for b in &blocks {
            assert!(b.cells.len() <= WIDEST_TABLE);
            let total: usize = b.spans.iter().map(|s| s.text.len()).sum();
            assert!(total <= LONGEST_BLOCK);
            assert!(torture.is_char_boundary(b.raw.start));
            assert!(torture.is_char_boundary(b.raw.end));
        }
    }
}
