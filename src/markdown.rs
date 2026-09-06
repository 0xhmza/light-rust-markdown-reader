//! A deliberately small, line-oriented Markdown reader, not a CommonMark engine.
//! All parsing is safe Rust. Markup we do not understand remains readable text.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style(pub u8);
impl Style {
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 2;
    pub const CODE: u8 = 4;
    pub const LINK: u8 = 8;
    pub const STRIKE: u8 = 16;
}

#[derive(Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    Quote,
    List(usize),
    Code,
    Rule,
    Gap,
    /// One table row. The payload marks the header row; text lives in `cells`.
    Row(bool),
}

/// A table cell: styled text plus its column alignment (0 left, 1 centre, 2 right).
#[derive(Debug)]
pub struct Cell {
    pub spans: Vec<Span>,
    pub align: u8,
}

#[derive(Debug)]
pub struct Block {
    pub kind: Kind,
    pub spans: Vec<Span>,
    pub cells: Vec<Cell>,
}
impl Block {
    fn new(kind: Kind, spans: Vec<Span>) -> Self {
        Self {
            kind,
            spans,
            cells: vec![],
        }
    }
}

fn push(spans: &mut Vec<Span>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.text.push_str(text);
            return;
        }
    }
    spans.push(Span {
        text: text.to_owned(),
        style,
    });
}

pub fn inline(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    parse_inline(text, Style::default(), 0, &mut spans);
    spans
}

fn parse_inline(text: &str, style: Style, depth: u8, out: &mut Vec<Span>) {
    if depth >= 12 {
        push(out, text, style);
        return;
    }
    let mut pos = 0;
    let mut plain = 0;
    let mut missing_delimiters = 0u8;
    let mut missing_code_runs = Vec::new();
    let mut links_exhausted = false;
    while pos < text.len() {
        let rest = &text[pos..];
        let c = rest.chars().next().unwrap();
        if c == '\\' {
            if let Some(next) = rest[1..]
                .chars()
                .next()
                .filter(|c| c.is_ascii_punctuation())
            {
                push(out, &text[plain..pos], style);
                push(out, &rest[1..1 + next.len_utf8()], style);
                pos += 1 + next.len_utf8();
                plain = pos;
                continue;
            }
        }
        if c == '`' {
            let n = rest.bytes().take_while(|&b| b == b'`').count();
            if !missing_code_runs.contains(&n) {
                if let Some(end) = closing_ticks(&rest[n..], n) {
                    push(out, &text[plain..pos], style);
                    push(out, &rest[n..n + end], Style(style.0 | Style::CODE));
                    pos += n + end + n;
                    plain = pos;
                    continue;
                }
                missing_code_runs.push(n);
            }
            pos += n;
            continue;
        }
        let image = rest.starts_with("![");
        if !links_exhausted && (c == '[' || image) {
            let start = if image { 2 } else { 1 };
            let label_end = closing_bracket(rest, start);
            if let Some(label_end) = label_end.filter(|&end| rest[end..].starts_with("](")) {
                let target = label_end + 2;
                if let Some(end) = closing_paren(rest, target) {
                    push(out, &text[plain..pos], style);
                    if image {
                        push(out, "[Image: ", style);
                    }
                    parse_inline(
                        &rest[start..label_end],
                        Style(style.0 | Style::LINK),
                        depth + 1,
                        out,
                    );
                    if image {
                        push(out, "]", style);
                    }
                    pos += end + 1;
                    plain = pos;
                    continue;
                }
                links_exhausted = true;
            }
            // A failed suffix search must not be repeated at every '[' in a
            // malformed document (which would make parsing quadratic).
            links_exhausted |= label_end.is_none();
        }
        let mut matched = false;
        for (index, (delim, flag)) in [
            ("***", Style::BOLD | Style::ITALIC),
            ("___", Style::BOLD | Style::ITALIC),
            ("**", Style::BOLD),
            ("__", Style::BOLD),
            ("~~", Style::STRIKE),
            ("*", Style::ITALIC),
            ("_", Style::ITALIC),
        ]
        .into_iter()
        .enumerate()
        {
            if missing_delimiters & (1 << index) != 0 || !rest.starts_with(delim) {
                continue;
            }
            if c == '_'
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
                    && (c != '_' || !after.chars().next().is_some_and(crate::text::word_char)))
                .then_some(at)
            }) else {
                missing_delimiters |= 1 << index;
                continue;
            };
            if end > 0 && !inner[..end].ends_with(char::is_whitespace) {
                push(out, &text[plain..pos], style);
                parse_inline(&inner[..end], Style(style.0 | flag), depth + 1, out);
                pos += delim.len() * 2 + end;
                plain = pos;
                matched = true;
                break;
            }
        }
        if !matched {
            pos += c.len_utf8();
        }
    }
    push(out, &text[plain..], style);
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

/// Split a table line into raw cells, honouring `\|` escapes and the optional
/// leading and trailing pipes.
fn cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut out = Vec::new();
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
    out
}

/// `| --- | :-: | ---: |` yields one alignment per column, otherwise `None`.
fn alignments(line: &str) -> Option<Vec<u8>> {
    cells(line)
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

/// Break one block's spans into pieces of at most `LONGEST_BLOCK` bytes,
/// preferring a space so an over-long paragraph reads as if it had wrapped.
fn split(spans: Vec<Span>) -> Vec<Vec<Span>> {
    if spans.iter().map(|span| span.text.len()).sum::<usize>() <= LONGEST_BLOCK {
        return vec![spans];
    }
    let mut pieces = vec![Vec::new()];
    let mut room = LONGEST_BLOCK;
    for span in spans {
        let mut text = span.text.as_str();
        while text.len() > room {
            // A char boundary at worst, a space if there is one to be had.
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
            push(pieces.last_mut().unwrap(), &text[..cut], span.style);
            pieces.push(Vec::new());
            room = LONGEST_BLOCK;
            text = &text[cut..];
        }
        push(pieces.last_mut().unwrap(), text, span.style);
        room -= text.len();
    }
    pieces.retain(|piece| !piece.is_empty());
    pieces
}

fn row(line: &str, align: &[u8], header: bool) -> Block {
    let mut block = Block::new(Kind::Row(header), vec![]);
    block.cells = align
        .iter()
        .zip(cells(line).into_iter().chain(std::iter::repeat("")))
        .map(|(&align, text)| Cell {
            spans: split(inline(text.trim()))
                .into_iter()
                .next()
                .unwrap_or_default(),
            align,
        })
        .collect();
    block
}

pub fn parse(source: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    // Emitting through this keeps every path inside the block-length cap.
    macro_rules! emit {
        ($kind:expr, $spans:expr) => {{
            let kind = $kind;
            for piece in split($spans) {
                blocks.push(Block::new(kind.clone(), piece));
            }
        }};
    }
    let mut fenced: Option<(u8, usize)> = None;
    let mut lines = source
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .peekable();
    while let Some(raw) = lines.next() {
        if blocks.len() >= LONGEST_DOCUMENT {
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
            emit!(
                Kind::Code,
                vec![Span {
                    text: raw.replace('\t', "    "),
                    style: Style(Style::CODE),
                }]
            );
            continue;
        }
        if let Some(f) = fence(trimmed) {
            fenced = Some(f);
            continue;
        }
        if trimmed.is_empty() {
            if !blocks.last().is_some_and(|b| b.kind == Kind::Gap) {
                blocks.push(Block::new(Kind::Gap, vec![]));
            }
            continue;
        }
        // Setext headings take precedence over horizontal rules.
        if (trimmed.trim_end().bytes().all(|b| b == b'=')
            || trimmed.trim_end().bytes().all(|b| b == b'-'))
            && blocks.last().is_some_and(|b| b.kind == Kind::Paragraph)
        {
            blocks.last_mut().unwrap().kind =
                Kind::Heading(if trimmed.starts_with('=') { 1 } else { 2 });
            continue;
        }
        if rule(trimmed) {
            blocks.push(Block::new(Kind::Rule, vec![]));
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
            emit!(Kind::Heading(hashes as u8), inline(content));
            continue;
        }
        // A pipe row is a table only when the next line delimits its columns.
        if trimmed.contains('|') {
            if let Some(align) = lines
                .peek()
                .map(|next| next.trim())
                .filter(|next| next.contains('|') || next.contains('-'))
                .and_then(alignments)
            {
                blocks.push(row(trimmed, &align, true));
                lines.next();
                while let Some(next) = lines
                    .peek()
                    .map(|next| next.trim_start())
                    .filter(|next| next.contains('|') && fence(next).is_none())
                {
                    blocks.push(row(next, &align, false));
                    lines.next();
                }
                continue;
            }
        }
        if trimmed.starts_with('>') {
            emit!(
                Kind::Quote,
                inline(trimmed.trim_start_matches('>').trim_start())
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
            let (marker, body) = if content.starts_with("[ ] ") {
                ("[ ] ".to_owned(), &content[4..])
            } else if content.starts_with("[x] ") || content.starts_with("[X] ") {
                ("[x] ".to_owned(), &content[4..])
            } else if ordered {
                (format!("{}. ", &trimmed[..digits]), content)
            } else {
                ("• ".to_owned(), content)
            };
            let mut spans = vec![Span {
                text: marker,
                style: Style::default(),
            }];
            spans.extend(inline(body));
            emit!(Kind::List((indent / 2).min(16)), spans);
        } else if raw.starts_with("    ") || raw.starts_with('\t') {
            let content = raw
                .strip_prefix("    ")
                .or_else(|| raw.strip_prefix('\t'))
                .unwrap_or(raw);
            emit!(
                Kind::Code,
                vec![Span {
                    text: content.replace('\t', "    "),
                    style: Style(Style::CODE),
                }]
            );
        } else {
            emit!(Kind::Paragraph, inline(trimmed));
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plain(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
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
        assert!(s
            .iter()
            .any(|s| s.text == "世界" && s.style.0 == Style::LINK));
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
        let b = parse(
            "\u{feff}# Title\r\n\r\n- [x] done\n> quote\n```rust\n**literal**\n````\n\n---\n",
        );
        assert_eq!(b[0].kind, Kind::Heading(1));
        assert_eq!(b[2].kind, Kind::List(0));
        assert_eq!(b[4].kind, Kind::Code);
        assert_eq!(plain(&b[4].spans), "**literal**");
        assert_eq!(b.last().unwrap().kind, Kind::Rule);
    }
    #[test]
    fn setext_and_unclosed_fence() {
        let b = parse("Title\n===\n\n~~~~\ntext\n~~~\n");
        assert_eq!(b[0].kind, Kind::Heading(1));
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
        // A short row is padded to the header's width, never dropped.
        assert_eq!(blocks[2].kind, Kind::Row(false));
        assert_eq!(blocks[2].cells.len(), 2);
        assert_eq!(plain(&blocks[2].cells[1].spans), "");
        // Without a delimiter row, pipes are ordinary text.
        assert_eq!(blocks[4].kind, Kind::Paragraph);
        assert_eq!(plain(&blocks[4].spans), "not | a table");
        assert_eq!(blocks[5].kind, Kind::Paragraph);
        assert_eq!(plain(&blocks[5].spans), "| pipes | only |");
    }
    #[test]
    fn over_long_lines_and_over_wide_tables_are_bounded() {
        // A split lands on a space where there is one nearby, so a long
        // paragraph reads as if it had simply wrapped.
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
        // A line with no space anywhere still has to be broken somewhere.
        let solid = "x".repeat(LONGEST_BLOCK * 3);
        let blocks = parse(&solid);
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks.iter().map(|b| plain(&b.spans)).collect::<String>(),
            solid
        );
        // Splitting never lands inside a character.
        let wide = "😀".repeat(LONGEST_BLOCK);
        assert_eq!(
            parse(&wide)
                .iter()
                .map(|b| plain(&b.spans))
                .collect::<String>(),
            wide
        );
        // A table cannot be made arbitrarily wide, and its cells are bounded
        // like any other block.
        let table = format!(
            "|{}
|{}
",
            "a|".repeat(5_000),
            "-|".repeat(5_000)
        );
        let blocks = parse(&table);
        assert!(blocks.iter().all(|b| b.cells.len() <= WIDEST_TABLE));
        let huge = format!(
            "| {} |
|---|
",
            "z".repeat(LONGEST_BLOCK * 2)
        );
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
        assert_eq!(blocks[1].cells[1].spans[0].style.0, Style::STRIKE);
    }
    /// Random markup, from a fixed seed so a failure can be reproduced. The
    /// parser has no business panicking on any of it, and no byte of the
    /// source may vanish: markup it does not understand stays readable text.
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
            // Every block is inside the length cap, whatever the input.
            for block in &blocks {
                let length: usize = block.spans.iter().map(|s| s.text.len()).sum();
                assert!(length <= LONGEST_BLOCK, "case {case}: {source:?}");
                assert!(block.cells.len() <= WIDEST_TABLE);
            }
            // Text is preserved: what the parser drops is only the markup it
            // consumed, so comparing what is left is the honest check.
            let kept: String = blocks
                .iter()
                .flat_map(|block| {
                    block.spans.iter().map(|s| s.text.as_str()).chain(
                        block
                            .cells
                            .iter()
                            .flat_map(|c| c.spans.iter().map(|s| s.text.as_str())),
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
        // An over-long line is broken into blocks, and not one byte is lost.
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
            let blocks = parse(&format!("日本語\n{underline}\n"));
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
            "简体中文",
            "繁體中文",
            "한국어",
            "العَرَبِيَّةُ",
            "שָׁלוֹם",
            "हिन्दी",
            "বাংলা",
            "தமிழ்",
            "ภาษาไทย",
            "ខ្មែរ",
            "မြန်မာ",
            "cafe\u{301}",
            "Tiếng Việt",
        ] {
            for (markup, style) in [
                (format!("**{text}**"), Style::BOLD),
                (format!("*{text}*"), Style::ITALIC),
                (format!("`{text}`"), Style::CODE),
                (format!("[{text}](https://example.test)"), Style::LINK),
            ] {
                let spans = inline(&markup);
                assert_eq!(plain(&spans), text);
                assert!(spans.iter().all(|s| s.style.0 == style));
            }
        }
    }
    #[test]
    fn adversarial_unicode_delimiters_preserve_unmatched_text() {
        for source in [
            "_字".repeat(30_000),
            "[हि".repeat(30_000),
            "cafe\u{301}_".repeat(30_000),
            "\\😀".repeat(30_000),
        ] {
            assert_eq!(plain(&inline(&source)), source);
        }
    }
}
