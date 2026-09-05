# icofon

Build an icon font and a matching stylesheet from a folder of SVG files.

```bash
icofon build ./icons -o dist
```

That reads every `.svg` under `./icons` and writes:

```
16 icons
  dist/icons.woff2             3.1 KB
  dist/icons.woff              3.5 KB
  dist/icons.ttf               5.1 KB
  dist/icons.css               1.3 KB
  dist/index.html              35.5 KB
  icons/icofon.json            1.4 KB
```

| File | What it is |
| --- | --- |
| `icons.woff2` | the font a browser will actually use — brotli, smallest by far |
| `icons.woff` | deflate, the fallback for anything older than 2016 |
| `icons.ttf` | uncompressed, for desktop apps and non-web tooling |
| `icons.css` | `@font-face` listing all three, plus one class per icon |
| `index.html` | a searchable preview of every icon |
| `icofon.json` | records each icon's codepoint so it never moves — commit it |

The stylesheet offers the formats smallest first, so a browser takes `woff2`
and never downloads the rest:

```css
@font-face {
  font-family: 'icons';
  src: url('icons.woff2') format('woff2'),
       url('icons.woff') format('woff'),
       url('icons.ttf') format('truetype');
}
```

Drop the first two on a page and the icons are available as CSS classes:

```html
<link rel="stylesheet" href="font.css">
<span class="icon-arrow-left"></span>
```

## Install

Homebrew — installs a prebuilt binary, so there is nothing to compile:

```bash
brew install jwo1f/tap/icofon
```

crates.io:

```bash
cargo install icofon
```

From a clone:

```bash
cargo install --path .
```

## Commands

```
icofon build [SOURCE]   Build the fonts, stylesheet and preview page
icofon check [SOURCE]   Convert every icon and report, writing nothing
icofon watch [SOURCE]   Build, then rebuild whenever an icon changes
icofon init  [SOURCE]   Write an icofon.toml
```

`check` is the one for CI: it fails on any icon that cannot become a glyph and
writes nothing.

## icofon.toml

Anything you would otherwise retype belongs in a config file. `icofon init`
writes one, and every command looks for it in the working directory and each
directory above, so a build behaves the same from anywhere in the project.

```toml
source  = "icons"
out     = "dist"
name    = "icons"
formats = ["woff2", "woff", "ttf"]
prefix  = "icon"
```

Paths in the file resolve relative to the file, not to where you ran the
command. Flags override the file; the file overrides the defaults.

## CSS classes

By default the stylesheet matches **any** class starting with the prefix:

```css
[class^="icon-"],
[class*=" icon-"] { font-family: 'font'; ... }
.icon-arrow-left::before { content: "\e900"; }
```

One class per icon, and nothing to remember. The cost is that the whole
`icon-*` namespace now belongs to the font: a class of your own that happens to
start the same way — a `.icon-button` wrapper, a utility class from a framework
— quietly picks up the icon font.

`--base-class` trades the shorthand for a scope. The prefix becomes a class in
its own right, and every rule requires it:

```css
.icon { font-family: 'font'; ... }
.icon.icon-arrow-left::before { content: "\e900"; }
```

```html
<span class="icon icon-arrow-left"></span>
```

Nothing is claimed by name alone, so your own `icon-*` classes are left alone.
Use it when the font is dropped into a codebase you do not fully control, or
when the prefix is a common word. `--prefix` changes the word itself and both
styles follow it, so `--prefix ico --base-class` gives `class="ico ico-arrow-left"`.

## What it does with your SVGs

- **Any SVG, not just single paths.** Groups, transforms, `<rect>`, `<circle>`,
  `<polygon>` and friends are all flattened into one outline.
- **Strokes are converted to outlines**, so stroke-drawn icon sets (Feather,
  Lucide, Tabler) come through as solid shapes. A font can only fill.
  `stroke-dasharray` is cut before the outline is taken, so a dotted rule stays
  a row of dots rather than becoming a bar.
- **A shape with no inside is dropped.** SVG fills a path black unless told
  otherwise, so a line drawn with only a `stroke` arrives carrying a black fill
  as well. It paints nothing — a line has no inside — but counted as a color it
  would cost a one-color icon the CSS `color`.
- **Cubic curves become quadratics**, which is all TrueType can store.
- **`fill-rule="evenodd"` is honored.** Fonts only do non-zero winding, so
  even-odd contours are re-oriented by nesting depth; holes stay holes instead
  of filling in.
- **Overlapping shapes are unioned, not cancelled.** Everything an icon is built
  from lands in one non-zero-filled path, where two contours that happen to turn
  opposite ways subtract instead of adding. Which way round the artwork drew
  something is not part of what it meant, so before shapes are merged each
  contour — together with everything nested inside it — is turned the same way.
  Reversing a whole nest at once leaves what it fills untouched, so shapes only
  ever add up. Without it, an icon fattened by stroking a shape in its own fill
  color comes out hollow, a mark laid over a panel eats a hole through it, and a
  gear whose teeth and lettering are one `<path>` drawn against each other loses
  the teeth into the rim.
- **White and faint washes are treated as paper, not ink.** A glyph has neither
  color nor opacity, so a white shape — or a shape at low opacity used as a
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
- **Colors are kept.** Every color the artwork names is emitted as a COLR/CPAL
  layer, so a card logo, a brand badge or a folder of file-type icons renders as
  drawn. Layers painted with `currentColor` use the palette entry reserved for
  the text color, so they follow CSS `color` while their neighbors stay fixed.
  `--color` changes this — see below.
- **The viewBox height maps onto one em.** An icon drawn edge to edge in its
  viewBox renders exactly `1em` tall. Width is scaled to match and becomes the
  glyph's advance, so non-square icons keep their proportions.

The em box runs from 200 units below the baseline to 800 above it, which puts
icons at the height that lines up with running text without extra CSS.

## Color

The artwork is taken at its word. A color it names is a choice and is kept; a
shape painted `currentColor` is the way to ask for something that follows the
CSS `color` of whatever the icon sits in.

```
user.svg          fill="currentColor"     -> plain glyph, follows CSS color
ruby.svg          fill="#e53935"          -> COLR: stays Ruby red
mastercard.svg    red + orange + white    -> COLR layers, keeps its colors
circle-check.svg  blue disc + currentColor tick
                                          -> COLR: disc stays blue, tick follows CSS color
```

Color icons still carry the flattened monochrome outline as their base glyph,
which is what a renderer without COLR support falls back to. The preview page
marks them "fixed color" so it is clear which ones will not follow your CSS.

Gradients are reduced to their first stop, since a layer is one flat color.

### `--color`

Not every set means its colors, so `--color` says how literally to read them.

| | |
| --- | --- |
| `keep` | The default, above: every named color is kept, `currentColor` recolors. |
| `recolor-single` | Also treat an icon drawn in one lone color as if that color had been `currentColor`. |
| `recolor` | Drop color entirely. Every icon is a plain glyph that follows CSS `color`. |

`recolor-single` suits a set drawn flat in a single black or gray — the color
there is a default rather than a choice, and an icon frozen in a mid gray
disappears against a background of it. It only ever gives up a *lone* color: an
icon that uses two or more still keeps them, because what several colors buy is
the *relationship* between them (the white lettering on a dark badge, the three
panels of a card logo), which flattening destroys.

```bash
icofon build ./icons -o dist --color recolor-single
```

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
Fix them, or pass --on-error skip to leave them out.
```

The cases caught are a **bitmap wrapped in an SVG** — a PNG pasted out of a
design tool, which would otherwise come through as a solid rectangle — artwork
shaped by a **`<mask>`**, which decides per-pixel how much of each shape
survives and so cannot be reduced to an outline, and a file that yields
**nothing drawable at all**.

`--on-error skip` builds the font without them and lists what it left out on
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

Each record is keyed by the icon's path inside the folder, and carries the name
that path was given:

```json
{
  "icons": {
    "arrows/left.svg": { "name": "arrows-left", "codepoint": "e900" }
  }
}
```

The path is the key rather than the name because a name is not always the file's
alone: two files whose names reduce to the same slug are told apart by a number,
and that number depends on what else is in the folder. A path does not move when
a neighbour appears, so neither does the codepoint. A manifest written by icofon
0.3 or earlier is keyed by name; it is read as it stands and rewritten under
paths on the next build, so no codepoint moves on the way across.

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

`index.html` lists every icon in a grid, grouped by subfolder, each card
showing the glyph, its name, its CSS class and its codepoint. Search filters on
all of those — type a name, a class, a hex code or a folder — and groups that
lose all their icons collapse with them. Clicking a class name copies it;
clicking anywhere else on a card opens the icon.

The title scrolls away; the toolbar under it sticks to the top and carries:

- **Search** over name, class, codepoint and folder. `/` jumps to it, `Esc`
  clears it.
- **Ink** — a light/dark switch and a color palette with a free-form picker, so
  icons can be checked on both backgrounds and in any color. The active color
  is always shown as a hex. Only the glyphs recolor; the labels stay readable.
  Both choices are remembered.
- **Color** — filter by how much CSS `color` can change: *Recolorable* (one
  color, and you set it), *Partly fixed* (one part follows your color, the rest
  are the artwork's), or *Fixed* (every color is the artwork's, so your CSS does
  nothing).
- **Folder** — narrow to one subfolder.

The three filters combine, so "fixed-color icons in `payment`" is two clicks.

Each of them is drawn only where it does something. A set with nothing painted
`currentColor` gets no palette, because nothing on the page would move; a set
that is all one color bucket gets no chips, because a chip that matches every
icon is not a filter — the page says which bucket it is once, beside the glyph
count, and the cards drop the note they would all have carried. One subfolder
holding every icon gets no folder picker: both entries are the same list.

### Opening one icon

Clicking a card opens that icon on its own. The glyph fills a square stage the
full height of the dialog, drawn inside its em box with its baseline marked, so
where it sits in a line of text is visible rather than guessed at, and a wide
icon shows how far past the square it runs. Beside it are the folder, the name,
the codepoint and what CSS `color` can do to it, then three rows that each copy
one thing: the class attribute, a `<span>` that uses it, and the `content` a
`::before` rule needs.

Under those are the icons whose names are most like this one's, which is how a
set is browsed rather than searched — open `arrow-left` and `arrow-right` is one
click away. Names are matched a word at a time on the file part of the name,
with a longer spelling of a word — `arrow` against `arrows` — still counting;
sharing the first word, or the folder, counts for a little more. The list
ignores the filters, since a search narrow enough to open an icon from has
usually hidden everything that icon is like.

The arrow keys walk the set without closing the dialog, `Esc` closes it, and so
does a click outside it.

The page carries its own favicon as a `data:` URI and signs off with a byline
at the foot, so a preview is still one file plus its font — nothing beside it
to copy, and nothing to go missing when it is moved.

The toolbar keeps one height whether stuck or not. A sticky bar that resizes
shifts the content below it, the browser's scroll anchoring then compensates by
moving the scroll position, and the two fight each other on slow scrolls.

Icons are one em tall but may be many ems wide. Each card is told its icon's
aspect ratio and scales the glyph down to fit, so a wordmark 16 times wider than
it is tall sits inside its card instead of running off the page; anything wider
than 1.5× is labeled with its width, which explains why it renders small.

It links the generated stylesheet, so it renders with exactly the CSS a site
would use. Its own layout comes from the [Tailwind browser build][tw] loaded
from a CDN, which compiles utility classes at page load; that keeps the page
buildless, but it does mean the preview needs network access to look right. The
font and stylesheet have no such dependency.

Use `--no-preview` to skip it.

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
`map_pin.svg` both give `map-pin`. That stops the build, and every clash is
named at once:

```
Error: 1 icon name is claimed by more than one file:
  'map-pin'
      icons/map-pin.svg
      icons/map_pin.svg
Rename one file in each group, or pass --on-duplicate number to number them.
```

Renaming one of the files is the fix that stays fixed: the name is then part of
the file, and nothing else in the folder can take it away.

`--on-duplicate number` builds anyway. The first in sorted order keeps the plain
name and the rest are numbered from 2, each one reported:

```
'map-pin' is already taken by icons/map-pin.svg, so icons/map_pin.svg is called 'map-pin-2'
```

```
icons/map-pin.svg   ->  .icon-map-pin
icons/map_pin.svg   ->  .icon-map-pin-2
```

Numbering steps over names a real file already has, so an existing
`map-pin-2.svg` keeps its own. A number is recorded in the manifest against the
file that got it and stays with that file, so a third clashing icon — even one
that sorts ahead of both — takes the next free number instead of renumbering the
icons that were there first. Classes and codepoints already in use do not move —
`--no-manifest` has nothing to record them in, so there the numbering is back to
following sort order.

The number stays until the file is renamed, which is still the better answer:
`map-pin-2` says nothing about what the icon is.

## Options

```
icofon build [SOURCE] [OPTIONS]

  -o, --out <DIR>        Where to write (default: dist)
      --name <NAME>      Base file name and font family (default: the source folder's name)
      --formats <LIST>   Containers to write (default: woff2,woff,ttf)
      --prefix <PREFIX>  CSS class prefix (default: icon)
      --base-class       Require the prefix as its own class: class="icon icon-arrow-left"
      --no-preview       Skip index.html
      --manifest <PATH>  Codepoint manifest (default: icofon.json in the icon folder)
      --no-manifest      Assign codepoints from scratch every build; they will move
      --on-error <WHAT>  fail (default) or skip icons that cannot become a glyph
      --on-duplicate <WHAT>
                         fail (default) or number icons whose names collide
      --color <WHICH>    keep (default), recolor-single or recolor
      --start <HEX>      First codepoint to assign (default: e900)
      --config <PATH>    Read this file instead of looking for icofon.toml
```

Generated files link each other by relative path, so pointing `--css` or
`--html` at a different directory still resolves.

## Example

```bash
icofon build examples/icons -o dist --name "My Icons" --prefix ico
```

```
16 icons
  dist/My Icons.woff2          3.1 KB
  dist/My Icons.woff           3.5 KB
  dist/My Icons.ttf            5.1 KB
  dist/My Icons.css            1.3 KB
  dist/index.html              35.4 KB
  examples/icons/icofon.json   1.4 KB
```

## Contributing

Bug reports and pull requests are welcome at
<https://github.com/JWo1F/icofon>. `cargo test` covers the SVG conversion, the
codepoint manifest, the stylesheet and the preview page; a change to any of
those should come with a test that fails without it.

The generated stylesheet is built through the small CSS writer in
`src/css/sheet.rs` rather than by formatting strings, so selectors and values
are escaped as they are made. The preview page's own styling is plain CSS in
`assets/`, included at compile time — edit it as CSS, not as a Rust literal.

Run `cargo fmt` before committing. The project is formatted by rustfmt with
`tab_spaces = 2` (see `rustfmt.toml`), rather than Rust's default four.

Because a font is compiled output, the useful check on a conversion change is
what it does to a real icon set: build one before and after, and say in the
commit message how many icons changed and which. Builds are reproducible, so
the icons that should not have moved will be byte-identical.

## License

MIT — see [LICENSE](LICENSE). Free to use, modify, redistribute and ship in
commercial work. The one condition is attribution: keep the copyright notice
and the license text in any copy or substantial portion, including in forks and
modified versions, so the original stays credited.

Releasing a new version, and publishing it to Homebrew, is described in
[RELEASING.md](RELEASING.md).
