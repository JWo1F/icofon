# icofon

Build an icon font and a matching stylesheet from a folder of SVG files.

```bash
icofon ./folder font.ttf
```

That reads every `.svg` in `./folder`, writes `font.ttf`, and writes `font.css`
next to it. Drop both on a page and the icons are available as CSS classes:

```html
<link rel="stylesheet" href="font.css">
<span class="icon-arrow-left"></span>
```

## Install

```bash
cargo install --path .
```

## What it does with your SVGs

- **Any SVG, not just single paths.** Groups, transforms, `<rect>`, `<circle>`,
  `<polygon>` and friends are all flattened into one outline.
- **Strokes are converted to outlines**, so stroke-drawn icon sets (Feather,
  Lucide, Tabler) come through as solid shapes. A font can only fill.
- **Cubic curves become quadratics**, which is all TrueType can store.
- **`fill-rule="evenodd"` is honoured.** Fonts only do non-zero winding, so
  even-odd contours are re-oriented by nesting depth; holes stay holes instead
  of filling in.
- **The viewBox height maps onto one em.** An icon drawn edge to edge in its
  viewBox renders exactly `1em` tall. Width is scaled to match and becomes the
  glyph's advance, so non-square icons keep their proportions.

The em box runs from 200 units below the baseline to 800 above it, which puts
icons at the height that lines up with running text without extra CSS.

## Names and codepoints

The file name becomes the CSS class, lowercased and slugified:

| File | Class |
| --- | --- |
| `arrow-left.svg` | `.icon-arrow-left` |
| `Arrow Left.svg` | `.icon-arrow-left` |
| `zoom_in (2).svg` | `.icon-zoom_in-2` |

Codepoints are assigned in file-name order starting at `U+E900`, in the Private
Use Area. To pin one, prefix the file name with `u` (or `U+`) and the hex:

```
uE9F0-star.svg   ->  .icon-star   content: "\e9f0"
```

Pinned codepoints are reserved before anything is auto-assigned, so they never
collide. Two icons resolving to the same name, or to the same codepoint, is an
error rather than a silently dropped glyph.

## Options

```
icofon <INPUT> <OUTPUT> [OPTIONS]

  --css <PATH>          Stylesheet path (default: OUTPUT with a .css extension)
  --font-family <NAME>  Family name in the font and the CSS (default: output file name)
  --prefix <PREFIX>     CSS class prefix (default: icon)
  --start <HEX>         First codepoint to assign (default: e900)
```

The `url()` in the stylesheet is written relative to wherever the stylesheet
lands, so `--css` pointing at a different directory still resolves.

## Example

```bash
icofon examples/icons dist/icons.ttf --font-family "My Icons" --prefix ico
```

```
6 icons -> dist/icons.ttf + dist/icons.css
```
