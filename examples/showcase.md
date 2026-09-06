# A small window for your words

MDLite draws Markdown directly with Windows GDI. Open a file, read it, close it.

## Text, without the machinery

Normal text, **bold text**, *italic text*, ***both together***, and ~~struck-out text~~.
Use `inline code` for a name like `CreateWindowExW`. Links appear as [readable labels](https://example.com).

> A quiet background, readable contrast, and familiar Windows controls.
> Switch the palette from the Options menu, or press Ctrl+D.

## Lists and tasks

- No browser runtime
- No editing controls
  - Native fonts and native scrolling
  - A single executable
- [x] Handwritten Markdown parser
- [ ] Choose your own colors in mdlite.ini

1. Open a document with Ctrl+O.
2. Read using the wheel, arrows, Page Up, or Page Down.
3. Press F5 to reload a changed document.

## Code stays literal

```rust
fn main() {
    let message = "Hello, Windows!";
    // **Markdown** is not parsed inside a code block.
    println!("{message}");
}
```

---

### Unicode text

Grüße aus Berlin. Café, naïve, Ελληνικά, Українська, 日本語, 中文, 😀.

### Plain and predictable

An image is represented by its label: ![a landscape](landscape.png).
Unrecognized markup stays readable. No network requests are made for document content.

### A long paragraph

This paragraph is intentionally long enough to wrap as you resize the window. The viewer measures text once during layout and stores its line positions. Scrolling draws the visible rows and uses Windows to move pixels already on the screen. There is no animation loop, background watcher, or web renderer keeping the application busy while you read.

Setext heading
--------------

Escaped punctuation: \*literal asterisks\*. File names such as snake_case remain plain.

## Tables

| Pipeline | Draws | Chosen when |
|:---------|:-----:|------------:|
| GDI | Latin text | always, if it can |
| GDI+ | Greek, Cyrillic, CJK | GDI cannot fall back well |
| Direct2D | العربية, हिन्दी, ไทย | the script needs shaping |

Columns size themselves, alignment comes from the delimiter row, and the footer
names whichever pipeline drew this page.
