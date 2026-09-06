# Tables

A plain table. Columns size themselves to their widest cell.

| Pipeline | Handles | Cost |
| -------- | ------- | ---- |
| GDI | Latin text | lowest |
| GDI+ | Greek, Cyrillic, CJK | low |
| Direct2D | shaping, bidi, colour | highest |

Alignment comes from the delimiter row: left, centre, then right.

| Item | Qty | Price |
|:-----|:---:|------:|
| Apple | 3 | 1.50 |
| Kiwi | 12 | 0.25 |
| Pomegranate | 1 | 4.00 |

Cells keep inline markup, and a short row is padded rather than dropped.

| Style | Example | Note |
| ----- | ------- | ---- |
| Bold | **bold** | `code` too |
| Link | [target](https://example.test) |
| Script | العربية · 日本語 · 😀 | escalates to Direct2D |

A table wider than the window shrinks its columns and wraps inside them.

| Description | Detail |
| ----------- | ------ |
| A long first column that has to give up room so that the second column can still be read | Wrapping happens inside each column, never across the page. |

Pipes only make a table when the next line delimits the columns, so `a | b`
stays ordinary text.
