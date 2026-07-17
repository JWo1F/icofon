# icofon

Build an icon font and a matching stylesheet from a folder of SVG files.

```bash
icofon ./folder font.ttf
```

That reads every `.svg` in `./folder` and writes three files next to it:

| File | What it is |
| --- | --- |
| `font.ttf` | the icon font |
| `font.css` | `@font-face` plus one class per icon |
| `example.html` | a searchable preview of every icon |

and one file inside the icon folder itself:

| File | What it is |
| --- | --- |
| `icofon.json` | records each icon's codepoint so it never moves — commit it |

Drop the first two on a page and the icons are available as CSS classes:

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
- **White and faint washes are treated as paper, not ink.** A glyph has neither
  colour nor opacity, so a white shape — or a shape at low opacity used as a
  tint — cannot be drawn as itself. What it means depends on what is under it:
  over existing artwork it is a knock-out and becomes a hole (the tick cut out
  of a filled circle, a wordmark cut out of a brand panel), and over nothing it
  is background and is dropped, rather than painted as a solid block that buries
  the design. An icon drawn *entirely* in white, or entirely as a wash, is just
  a light icon and is still drawn.
- **`currentcolor` is accepted in any case.** SVG keywords are case-insensitive
  and design tools emit the lowercase spelling, but the underlying parser only
  matches `currentColor` and would otherwise drop the paint — turning a
  stroke-drawn icon into a blank glyph.
- **Multi-colour icons keep their colours.** An icon that uses more than one
  colour is emitted as COLR/CPAL layers, so a card logo or a brand badge renders
  as drawn. Layers painted with `currentColor` use the palette entry reserved
  for the text colour, so they still follow CSS `color` while their neighbours
  stay fixed. Everything else stays a plain glyph — see below.
- **The viewBox height maps onto one em.** An icon drawn edge to edge in its
  viewBox renders exactly `1em` tall. Width is scaled to match and becomes the
  glyph's advance, so non-square icons keep their proportions.

The em box runs from 200 units below the baseline to 800 above it, which puts
icons at the height that lines up with running text without extra CSS.

## Colour

Most icons should follow the CSS `color` of whatever they sit in, and by default
they do: an icon that uses a single colour — whether that is `currentColor` or a
hardcoded `#3F3C43` — is a plain monochrome glyph and recolours freely.

Colour is only used when an icon needs it, which is when it uses **two or more
colours**. What colour buys is the *relationship* between them: the white
lettering on a dark badge, the three panels of a card logo. Flattening destroys
that. A single flat colour has no such relationship, so pinning it would take
away recolouring and gain nothing — and an icon frozen in a mid grey disappears
against a dark background.

```
arrow-right.svg   fill="#3F3C43"          -> plain glyph, follows CSS color
user.svg          fill="currentColor"     -> plain glyph, follows CSS color
mastercard.svg    red + orange + white    -> COLR layers, keeps its colours
circle-check.svg  blue disc + currentColor tick
                                          -> COLR: disc stays blue, tick follows CSS color
```

Colour icons still carry the flattened monochrome outline as their base glyph,
which is what a renderer without COLR support falls back to. The preview page
marks them "fixed colour" so it is clear which ones will not follow your CSS.

Gradients are reduced to their first stop, since a layer is one flat colour.

## Icons that cannot become glyphs

A glyph is an outline, so some SVGs have nothing to trace. The build fails and
names every one of them at once, rather than shipping an icon that looks fine in
the folder and renders blank or as a black box on the page:

```
Error: 4 of 410 icons cannot be turned into a glyph:
  atol (icons/atol.svg)
      the artwork is a bitmap embedded in the SVG, and a glyph can only be an
      outline; re-export it as vector paths
  ...
Fix them, or pass --skip-errors to leave them out.
```

The cases caught are a **bitmap wrapped in an SVG** — a PNG pasted out of a
design tool, which would otherwise come through as a solid rectangle — artwork
shaped by a **`<mask>`**, which decides per-pixel how much of each shape
survives and so cannot be reduced to an outline, and a file that yields
**nothing drawable at all**.

`--skip-errors` builds the font without them and lists what it left out on
stderr.

What is *not* caught, because the shape still converts: gradients and partial
opacity flatten to solid fills, since a glyph is monochrome.

## Adding icons later

Builds are reproducible: the same icons compile to the same bytes every time,
so a font committed alongside its sources only changes when the icons do.

Codepoints are recorded in `icofon.json` inside the icon folder. On every build
an icon keeps the codepoint it already had, and only icons icofon has never seen
get a new one — so dropping `aardvark.svg` into a set does not shift everything
after it and break pages already using the font.

That makes `icofon.json` worth committing alongside the SVGs. Without it, the
guarantee is gone: codepoints would be reassigned in file-name order on every
build.

A codepoint is never recycled. Delete an icon and its codepoint stays reserved,
so a future icon cannot quietly inherit the meaning of the old one; the same
holds for a codepoint an icon vacates by being pinned elsewhere.

Use `--manifest <PATH>` to keep the file somewhere else, or `--no-manifest` to
opt out of stable codepoints entirely.

## Subfolders

A subfolder prefixes the names of the icons inside it, and groups them in the
preview page:

```
icons/
  check.svg            ->  .icon-check              (no heading)
  arrows/
    left.svg           ->  .icon-arrows-left        under "arrows"
    right.svg          ->  .icon-arrows-right       under "arrows"
  social/
    left.svg           ->  .icon-social-left        under "social"
```

So two folders can each hold a `left.svg` without clashing. Nesting keeps
going — `arrows/filled/up.svg` is `.icon-arrows-filled-up`.

The preview page shows only the file part of the name on each card, since the
heading above it already names the folder; the class underneath carries the
full name.

Moving an icon between folders renames it, and therefore gives it a new
codepoint — the old one is retired, not reused.

## The preview page

`example.html` lists every icon in a grid, grouped by subfolder, each card
showing the glyph, its name, its CSS class and its codepoint. Search filters on
all of those — type a name, a class, a hex code or a folder — and groups that
lose all their icons collapse with them. Clicking a class name copies it.

The header stays put while you scroll, shrinking to a compact bar once it
sticks. It carries:

- **Search** over name, class, codepoint and folder. `/` jumps to it, `Esc`
  clears it.
- **Ink** — a light/dark switch and a colour palette with a free-form picker, so
  icons can be checked on both backgrounds and in any colour. Only the glyphs
  recolour; the labels stay readable. Both choices are remembered.
- **Colour** — filter by how an icon behaves: *Follows CSS* (drawn entirely in
  `currentColor`), *One colour* (a single named colour, still recolourable), or
  *Multicolour* (keeps its own colours and ignores your CSS).
- **Folder** — narrow to one subfolder.

The three filters combine, so "multicolour icons in `payment`" is two clicks.

Icons are one em tall but may be many ems wide. Each card is told its icon's
aspect ratio and scales the glyph down to fit, so a wordmark 16 times wider than
it is tall sits inside its card instead of running off the page; anything wider
than 1.5× is labelled with its width, which explains why it renders small.

It links the generated stylesheet, so it renders with exactly the CSS a site
would use. Its own layout comes from the [Tailwind browser build][tw] loaded
from a CDN, which compiles utility classes at page load; that keeps the page
buildless, but it does mean the preview needs network access to look right. The
font and stylesheet have no such dependency.

Use `--html <PATH>` to put it somewhere else, or `--no-html` to skip it.

[tw]: https://tailwindcss.com/docs/installation/play-cdn

## Names and codepoints

The file name becomes the CSS class, lowercased and slugified, prefixed with
any subfolders it sits in:

| File | Class |
| --- | --- |
| `arrow-left.svg` | `.icon-arrow-left` |
| `Arrow Left.svg` | `.icon-arrow-left` |
| `zoom_in (2).svg` | `.icon-zoom-in-2` |
| `arrows/left.svg` | `.icon-arrows-left` |

New icons are assigned codepoints in file-name order starting at `U+E900`, in
the Private Use Area; existing icons keep whatever they already had. To pin one
explicitly, prefix the file name with `u` (or `U+`) and the hex:

```
uE9F0-star.svg   ->  .icon-star   content: "\e9f0"
```

Pinned codepoints are reserved before anything is auto-assigned, so they never
collide. A pin moves the icon it names and the manifest follows; a pin that
would take a codepoint already recorded for a *different* icon is an error, as
are two icons pinning the same codepoint.

## Duplicate names

Different file names can still slugify to the same thing — `map-pin.svg` and
`map_pin.svg` both give `map-pin`. Rather than failing, the first in sorted
order keeps the plain name and the rest are numbered, and each one is reported:

```
'map-pin' is already taken by icons/map-pin.svg, so icons/map_pin.svg is called 'map-pin-2'
```

```
icons/map-pin.svg   ->  .icon-map-pin
icons/map_pin.svg   ->  .icon-map-pin-2
```

Numbering starts at 2 and steps over names a real file already has, so an
existing `map-pin-2.svg` keeps its own name.

Treat the message as something to fix rather than a normal state. The numbering
depends on sort order, so adding a *third* colliding file that sorts before the
others renumbers the ones after it — which changes their names, and therefore
their codepoints. Renaming one of the originals is the durable fix.

## Options

```
icofon <INPUT> <OUTPUT> [OPTIONS]

  --css <PATH>          Stylesheet path (default: OUTPUT with a .css extension)
  --html <PATH>         Preview page path (default: example.html beside the stylesheet)
  --no-html             Skip the preview page
  --manifest <PATH>     Codepoint manifest path (default: icofon.json in the icon folder)
  --no-manifest         Do not read or write the manifest; codepoints then move as icons are added
  --skip-errors         Leave out icons that cannot become a glyph, listing them on stderr,
                        instead of failing the build
  --font-family <NAME>  Family name in the font and the CSS (default: output file name)
  --prefix <PREFIX>     CSS class prefix (default: icon)
  --start <HEX>         First codepoint to assign (default: e900)
```

Generated files link each other by relative path, so pointing `--css` or
`--html` at a different directory still resolves.

## Example

```bash
icofon examples/icons dist/icons.ttf --font-family "My Icons" --prefix ico
```

```
6 icons -> dist/icons.ttf + dist/icons.css + dist/example.html + examples/icons/icofon.json
```
