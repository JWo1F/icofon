//! A browsable preview page listing every icon in the generated font.

use kurbo::{PathEl, Shape};

use crate::css::Classes;
use crate::font::{ASCENDER, DESCENDER, Icon, UNITS_PER_EM};
use crate::svg::Coloring;

/// Tailwind's browser build compiles utility classes at runtime, so the preview
/// page needs no build step of its own.
const TAILWIND_CDN: &str = "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4";

/// Tailwind's own configuration: the dark-mode variant and the two font
/// families the page sets. Kept as CSS so it stays readable and editable as
/// CSS; nothing in it is generated.
const THEME_CSS: &str = include_str!("../assets/preview-theme.css");

/// The page's own styling, on top of Tailwind's utilities. Also entirely
/// static — it is included verbatim.
const PREVIEW_CSS: &str = include_str!("../assets/preview.css");

/// The icofon mark, used as the page's favicon.
///
/// Everything that does not survive a 16-pixel tab has been taken out of it —
/// the wordmark, the pen-tool nodes, the blur — so what is left is the badge
/// and the letter, which still read at that size. The ring keeps the badge's
/// silhouette on a dark tab strip, where the navy would otherwise vanish.
const MARK_SVG: &str = include_str!("../assets/mark.svg");

/// The same mark set beside the wordmark, for the byline at the foot of the
/// page. It is written into the document rather than linked, so that `ico`
/// takes its color from the surrounding text and follows the light/dark switch
/// with everything else.
const LOGO_SVG: &str = include_str!("../assets/logo.svg");

/// Where the byline points.
const HOMEPAGE: &str = "https://github.com/JWo1F/icofon";

/// Render `example.html`: a searchable grid of every icon, showing its glyph,
/// name, CSS class and codepoint.
///
/// `css_url` is the stylesheet's location relative to the page, so the preview
/// exercises exactly the same CSS a site would use.
pub fn render(icons: &[Icon], family: &str, classes: Classes<'_>, css_url: &str) -> String {
  let family = escape(family);
  // Whether saying how an icon is colored tells a reader anything. In a set of
  // one kind it does not, and the caption belongs above the grid instead.
  let mark_color = kinds_present(icons) > 1;
  let mut page = String::new();

  page.push_str(&format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{family} — icons</title>
<link rel="icon" href="data:image/svg+xml;base64,{favicon}">
<link rel="stylesheet" href="{css}">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;700&family=IBM+Plex+Mono:wght@400;500&display=swap">
<script src="{tailwind}"></script>
<style type="text/tailwindcss">
{theme_css}</style>
<style>
{preview_css}</style>
<script>
  // Set before first paint so the page does not flash the wrong background.
  // Light by default: icons are drawn for a white page, so that is the honest
  // first impression. The dark switch is there to check the other background.
  (() => {{
    let saved = null;
    try {{ saved = localStorage.getItem('icofon-theme'); }} catch {{}}
    document.documentElement.dataset.theme = saved || 'light';
  }})();
</script>
</head>
<body class="bg-white text-zinc-900 antialiased dark:bg-zinc-950 dark:text-zinc-100">
<div class="mx-auto max-w-6xl px-5 pb-20 sm:px-8">

<div class="flex items-end justify-between gap-6 pt-10 pb-5">
  <div class="min-w-0">
    <h1 class="font-display text-[clamp(2rem,5vw,2.75rem)] leading-[0.95] tracking-tight">{family}</h1>
    <p class="mt-2 font-mono text-[10px] tracking-[0.22em] text-zinc-500 uppercase dark:text-zinc-400">
      <span id="shown">{total}</span> <span class="text-zinc-300 dark:text-zinc-600">/</span> {total} glyphs{set_note}
    </p>
  </div>
  <div class="flex shrink-0 items-center gap-1 rounded-full border border-zinc-200 p-1 dark:border-zinc-800">
    <button type="button" data-theme-set="light" title="Light background"
            class="theme-btn size-7 cursor-pointer rounded-full text-[13px] leading-none">☀</button>
    <button type="button" data-theme-set="dark" title="Dark background"
            class="theme-btn size-7 cursor-pointer rounded-full text-[13px] leading-none">☾</button>
  </div>
</div>

<div id="sentinel" aria-hidden="true"></div>
<header id="bar" class="sticky top-0 z-20 -mx-5 px-5 sm:-mx-8 sm:px-8">
  <div class="pt-3 pb-3">

    <div class="flex flex-wrap items-center gap-x-5 gap-y-3">
      <span id="mini" class="font-display text-lg leading-none">{family}</span>
      <label class="group relative flex min-w-56 flex-1 items-center">
        <svg class="pointer-events-none absolute left-3 size-4 text-zinc-400" viewBox="0 0 20 20" fill="none"
             stroke="currentColor" stroke-width="1.6" aria-hidden="true">
          <circle cx="9" cy="9" r="6"/><path d="m17 17-3.6-3.6" stroke-linecap="round"/>
        </svg>
        <input id="search" type="search" placeholder="Search icons…" autocomplete="off" autofocus
               class="w-full rounded-full border border-zinc-200 bg-transparent py-2 pr-14 pl-9 text-sm
                      placeholder:text-zinc-400 focus:border-zinc-400 focus:outline-none
                      dark:border-zinc-800 dark:placeholder:text-zinc-500 dark:focus:border-zinc-600">
        <kbd class="pointer-events-none absolute right-3 rounded border border-zinc-200 px-1.5
                    py-0.5 font-mono text-[10px] text-zinc-400 dark:border-zinc-800
                    dark:text-zinc-500">/</kbd>
      </label>

{ink}    </div>
{filters}
  </div>
  <div class="hairline"></div>
</header>

<main class="pt-7">
"##,
        family = family,
        css = escape(css_url),
        favicon = base64(MARK_SVG.as_bytes()),
        tailwind = TAILWIND_CDN,
        theme_css = THEME_CSS,
        preview_css = PREVIEW_CSS,
        total = icons.len(),
        set_note = set_note(icons),
        ink = ink_controls(icons),
        filters = filter_row(icons),
    ));

  // Icons arrive sorted by group, so consecutive runs share a section.
  for (group, members) in group_runs(icons) {
    page.push_str(&section_open(group));
    for icon in members {
      page.push_str(&card(icon, classes, mark_color));
    }
    page.push_str("  </div>\n</section>\n");
  }

  page.push_str(&format!(
    r#"</main>

<p id="empty" hidden class="mt-10 text-center text-zinc-500 dark:text-zinc-400">
  No icons match that search.
</p>

<footer class="mt-16 flex justify-center border-t border-zinc-100 pt-8 dark:border-zinc-900">
  <a href="{home}" target="_blank" rel="noreferrer noopener"
     class="flex items-center gap-2.5 text-zinc-400 transition hover:text-zinc-600
            dark:text-zinc-600 dark:hover:text-zinc-400">
    <span class="font-mono text-[10px] tracking-[0.22em] uppercase">Built with</span>
    {logo}  </a>
</footer>

{detail}
<div id="toast" popover="manual" role="status" aria-live="polite"
     class="pointer-events-none fixed bottom-7 left-1/2 -translate-x-1/2 translate-y-3 rounded-lg
            bg-zinc-900 px-3.5 py-2 text-sm text-white opacity-0 transition
            dark:bg-zinc-100 dark:text-zinc-900"></div>

</div>
<script>
  // The font's own grid, so the dialog can place its guides and read its
  // numbers out of the same constants the font was built from.
  const EM = {{ units: {units}, ascender: {ascender}, descender: {descender} }};
{script}
</script>
</body>
</html>
"#,
    script = SCRIPT,
    home = HOMEPAGE,
    logo = LOGO_SVG,
    detail = detail(recolorable(icons)),
    units = UNITS_PER_EM,
    ascender = ASCENDER,
    descender = DESCENDER,
  ));

  page
}

/// The color buckets a reader can filter by.
///
/// All three sit on one axis — how much of the icon CSS `color` can change —
/// so the names alone say what each means without a legend.
const KINDS: [(&str, &str, &str); 3] = [
  ("single", "Recolorable", "one color, and CSS color sets it"),
  (
    "mixed",
    "Partly fixed",
    "one part follows CSS color; the other colors are fixed by the artwork",
  ),
  (
    "fixed",
    "Fixed",
    "every color is fixed by the artwork, so CSS color does nothing",
  ),
];

/// The color palette and picker, or nothing when they would do nothing.
///
/// They set the CSS `color` the glyphs sit in, which reaches an icon only
/// through a part painted `currentColor`. A set where every color is the
/// artwork's has no such part, so the controls would move a swatch ring around
/// and change nothing on the page.
fn ink_controls(icons: &[Icon]) -> String {
  if !recolorable(icons) {
    return String::new();
  }

  let swatches: String = SWATCHES
    .iter()
    .map(|color| {
      format!(
        "      <button type=\"button\" data-color=\"{color}\" title=\"{color}\"\n\
         \x20             class=\"swatch size-6 cursor-pointer rounded-full border \
         border-black/10 dark:border-white/15\" style=\"background:{color}\"></button>\n"
      )
    })
    .collect();

  format!(
    r##"      <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-2">
        <span class="font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase dark:text-zinc-500">Ink</span>
        <div id="swatches" class="flex flex-wrap items-center gap-1.5">
{swatches}        </div>
        <span class="mx-1 h-4 w-px bg-zinc-200 dark:bg-zinc-800"></span>
        <label class="picker-ring ring-1 ring-black/10 dark:ring-white/20" title="Choose any color">
          <input id="picker" class="ink-picker" type="color" value="#2563eb">
        </label>
        <span id="color-note" class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500"></span>
      </div>
"##
  )
}

/// The row of filters under the search box, or nothing when neither filter has
/// anything to offer. Its own margin would otherwise leave a gap where the
/// controls were.
fn filter_row(icons: &[Icon]) -> String {
  let kinds = kind_chips(icons);
  let folders = folder_chips(icons);
  if kinds.is_empty() && folders.is_empty() {
    return String::new();
  }

  let kinds = if kinds.is_empty() {
    String::new()
  } else {
    format!(
      r##"      <div class="flex items-center gap-2">
        <span class="font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase dark:text-zinc-500">Color</span>
        <div id="kinds" class="flex flex-wrap items-center gap-1">
{kinds}        </div>
      </div>
"##
    )
  };

  format!(
    "\n    <div class=\"mt-3 flex flex-wrap items-center gap-x-4 gap-y-2\">\n{kinds}{folders}    </div>\n"
  )
}

/// One chip per color bucket the set actually contains.
///
/// A single bucket is not a filter: every chip would either show everything or
/// hide everything, so a set that is all one kind gets no chips at all.
fn kind_chips(icons: &[Icon]) -> String {
  if kinds_present(icons) < 2 {
    return String::new();
  }

  let mut out = String::new();
  for (key, label, title) in KINDS {
    let count = icons.iter().filter(|i| kind_of(i) == key).count();
    if count == 0 {
      continue;
    }
    out.push_str(&format!(
            "          <button type=\"button\" data-kind=\"{key}\" title=\"{title}\"\n\
             \x20                   class=\"chip cursor-pointer rounded-full border border-zinc-200 px-2.5 \
             py-1 font-mono text-[11px] text-zinc-500 dark:border-zinc-800 \
             dark:text-zinc-400\">{label} <span class=\"text-zinc-300 dark:text-zinc-600\">{count}\
             </span></button>\n"
        ));
  }
  out
}

/// The folder picker, or nothing when there is nowhere else to go.
///
/// One folder that holds every icon is the same list under both entries, so the
/// picker only earns its place once a choice narrows something: two folders, or
/// one folder alongside icons sitting at the top level.
fn folder_chips(icons: &[Icon]) -> String {
  let mut folders: Vec<&str> = icons.iter().filter_map(|i| i.group.as_deref()).collect();
  folders.sort_unstable();
  folders.dedup();
  let ungrouped = icons.iter().any(|icon| icon.group.is_none());
  if folders.is_empty() || (folders.len() == 1 && !ungrouped) {
    return String::new();
  }

  let mut options = String::from("          <option value=\"\">All folders</option>\n");
  for folder in folders {
    let count = icons
      .iter()
      .filter(|i| i.group.as_deref() == Some(folder))
      .count();
    options.push_str(&format!(
      "          <option value=\"{f}\">{f} ({count})</option>\n",
      f = escape(folder)
    ));
  }

  format!(
    "      <div class=\"flex items-center gap-2\">\n\
         \x20       <span class=\"font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase \
         dark:text-zinc-500\">Folder</span>\n\
         \x20       <select id=\"folder\" class=\"cursor-pointer rounded-full border border-zinc-200 \
         bg-transparent py-1 pr-7 pl-2.5 font-mono text-[11px] text-zinc-500 focus:outline-none \
         dark:border-zinc-800 dark:text-zinc-400\">\n{options}        </select>\n      </div>\n"
  )
}

/// What a set that is all one kind is, said once beside the glyph count.
///
/// The per-card notes and the filter chips both go when every icon is the same
/// kind, so this is what is left to explain why the CSS `color` does nothing —
/// and it says it in the one place a reader is already looking.
fn set_note(icons: &[Icon]) -> String {
  if kinds_present(icons) != 1 {
    return String::new();
  }
  let label = match icons.first().map(|icon| icon.outline.coloring) {
    Some(Coloring::Fixed) => "color fixed by the artwork",
    Some(Coloring::Mixed) => "color partly fixed by the artwork",
    // The ordinary case: an icon that follows the CSS `color` needs no saying.
    _ => return String::new(),
  };
  format!("\n      <span class=\"text-zinc-300 dark:text-zinc-600\">·</span> {label}")
}

/// How many color buckets the set actually spans.
fn kinds_present(icons: &[Icon]) -> usize {
  KINDS
    .iter()
    .filter(|(key, _, _)| icons.iter().any(|icon| kind_of(icon) == *key))
    .count()
}

/// Which color bucket an icon belongs to.
fn kind_of(icon: &Icon) -> &'static str {
  match icon.outline.coloring {
    Coloring::Single => "single",
    Coloring::Mixed => "mixed",
    Coloring::Fixed => "fixed",
  }
}

/// Split the icons into consecutive runs sharing a group, preserving order.
fn group_runs(icons: &[Icon]) -> Vec<(Option<&str>, &[Icon])> {
  let mut runs = Vec::new();
  let mut rest = icons;
  while let Some(first) = rest.first() {
    let group = first.group.as_deref();
    let end = rest
      .iter()
      .position(|icon| icon.group.as_deref() != group)
      .unwrap_or(rest.len());
    let (run, remainder) = rest.split_at(end);
    runs.push((group, run));
    rest = remainder;
  }
  runs
}

/// Open a `<section>` for one group. Icons at the top level get a section with
/// no heading, so an icon folder without subfolders looks exactly as before.
fn section_open(group: Option<&str>) -> String {
  let heading = match group {
    Some(group) => format!(
      r#"  <h2 class="mb-3 font-mono text-sm font-medium text-zinc-500 dark:text-zinc-400">{}</h2>
"#,
      escape(group)
    ),
    None => String::new(),
  };
  format!(
    r#"<section class="icon-group mb-8" data-group="{group}">
{heading}  <div class="grid grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] gap-3">
"#,
    group = escape(group.unwrap_or_default()),
  )
}

/// Whether anything in the set follows the CSS `color` at all.
fn recolorable(icons: &[Icon]) -> bool {
  icons
    .iter()
    .any(|icon| matches!(icon.outline.coloring, Coloring::Single | Coloring::Mixed))
}

/// What the font knows about one glyph's outline, in the font's own units.
struct Metrics {
  /// Tight bounds of the ink — x0 y0 x1 y1 — or none when nothing is drawn.
  bounds: Option<[i32; 4]>,
  /// Closed loops in the outline, and the anchor points that make them up.
  /// Together they are how heavy the glyph is to draw, which is the one thing
  /// about an outline a page pays for.
  contours: usize,
  nodes: usize,
}

/// Measure a glyph the way the dialog reports it.
///
/// The bounds are the curves' own, not the integer box the compiled glyph
/// carries, so they can sit half a unit off what a font inspector prints. That
/// is below what any of these numbers is read for.
fn measure(icon: &Icon) -> Metrics {
  let path = &icon.outline.path;
  let bounds = path.segments().next().map(|_| {
    let box_ = path.bounding_box();
    [
      box_.x0.round() as i32,
      box_.y0.round() as i32,
      box_.x1.round() as i32,
      box_.y1.round() as i32,
    ]
  });
  let mut contours = 0;
  let mut nodes = 0;
  for element in path.elements() {
    match element {
      PathEl::MoveTo(_) => {
        contours += 1;
        nodes += 1;
      }
      // A close draws no new point: it returns to the one the contour opened on.
      PathEl::ClosePath => {}
      _ => nodes += 1,
    }
  }
  Metrics {
    bounds,
    contours,
    nodes,
  }
}

/// The file an icon was drawn from, without the path to the icon folder — that
/// part is the same for every icon and says nothing, and it is the one part
/// that differs between machines.
fn file_name(icon: &Icon) -> String {
  icon
    .source
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default()
}

fn note(text: &str) -> String {
  format!(
    r#"
        <code class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500">{text}</code>"#
  )
}

fn card(icon: &Icon, classes: Classes<'_>, mark_color: bool) -> String {
  // What you would write in a `class` attribute, which is what the button
  // copies, and the selector that matches it, which is what it displays.
  let class = classes.attr(&icon.name);
  let selector = classes.selector(&icon.name).to_string();
  let code = icon.codepoint as u32;
  // How many ems wide the glyph is. A wide icon would otherwise run straight
  // out of its card, so the stylesheet divides the display size by this.
  let aspect = f64::from(icon.outline.advance) / f64::from(crate::font::UNITS_PER_EM);
  // Say when an icon will not simply follow the CSS `color`, and how far that
  // goes: a mixed icon still has one part that does. A set that is all one kind
  // is told once, above the grid, rather than on every card — a label every
  // card carries draws no distinction and is only noise.
  let color_note = match icon.outline.coloring {
    _ if !mark_color => String::new(),
    Coloring::Single => String::new(),
    Coloring::Mixed => note("partly fixed"),
    Coloring::Fixed => note("fixed"),
  };
  // What the dialog reports about the glyph. Carried on the card because the
  // dialog is filled from whichever card was clicked, so the page holds one
  // copy of every icon's facts rather than a second table of them.
  let metrics = measure(icon);
  let bounds = match metrics.bounds {
    Some([x0, y0, x1, y1]) => format!("{x0} {y0} {x1} {y1}"),
    None => String::new(),
  };
  let file = match &icon.group {
    Some(group) => format!("{group}/{}", file_name(icon)),
    None => file_name(icon),
  };
  let width_note = if aspect >= 1.5 {
    format!(
      r#"
        <code class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500">{aspect:.1}× wide</code>"#
    )
  } else {
    String::new()
  };

  format!(
    r#"    <figure data-search="{search}" data-kind="{kind}" data-group="{group}" data-name="{full}"
            data-code="{code:04x}" data-bbox="{bounds}" data-outline="{contours} {nodes}"
            data-file="{file}" style="--aspect:{aspect:.3}"
            class="card m-0 flex flex-col items-center gap-3 rounded-xl border border-zinc-200
                   px-3 pt-5 pb-3.5 text-center dark:border-zinc-800">
      <button type="button" class="open cursor-pointer" aria-label="Open {name}">
        <span class="glyph {class} leading-none" aria-hidden="true"></span>
      </button>
      <figcaption class="flex w-full flex-col items-center gap-1">
        <span class="card-name max-w-full text-[13px] font-medium break-all">{name}</span>{width_note}{color_note}
        <button type="button" data-copy="{class}" title="Copy class name"
                class="name max-w-full cursor-pointer rounded-md px-1.5 py-0.5 font-mono text-[12px]
                       break-all text-zinc-500 hover:bg-blue-600/10 hover:text-blue-700
                       dark:text-zinc-400 dark:hover:bg-blue-400/15
                       dark:hover:text-blue-300">{selector}</button>
        <code class="font-mono text-[11px] text-zinc-400 dark:text-zinc-500">U+{code:04X}</code>
      </figcaption>
    </figure>
"#,
    // Searchable on name, class, hex code and group, in one flat haystack.
    search = escape(
      &[
        icon.name.as_str(),
        class.as_str(),
        &format!("{code:04x}"),
        icon.group.as_deref().unwrap_or_default(),
        match icon.outline.coloring {
          Coloring::Single => "",
          Coloring::Mixed => "partly fixed mixed multicolor",
          Coloring::Fixed => "fixed color multicolor",
        },
      ]
      .iter()
      .filter(|part| !part.is_empty())
      .cloned()
      .collect::<Vec<_>>()
      .join(" ")
    ),
    class = escape(&class),
    selector = escape(&selector),
    name = escape(&icon.label),
    // The whole name, group and all: the dialog matches icons on their words,
    // and the card's own title is only the leaf.
    full = escape(&icon.name),
    code = code,
    aspect = aspect,
    bounds = bounds,
    contours = metrics.contours,
    nodes = metrics.nodes,
    file = escape(&file),
    width_note = width_note,
    color_note = color_note,
    kind = kind_of(icon),
    group = escape(icon.group.as_deref().unwrap_or_default()),
  )
}

/// Preset colors for the preview. Black and white first, since checking an
/// icon against each background is the common case, then a few hues. Not a
/// palette editor — the picker beside them covers anything else.
const SWATCHES: [&str; 9] = [
  "#000000", "#ffffff", "#71717a", "#2563eb", "#0d9488", "#16a34a", "#ca8a04", "#dc2626", "#9333ea",
];

/// The detail dialog: one icon, large, with everything the card had to
/// abbreviate spelled out beside it.
///
/// The markup is empty -- a set of a thousand icons would otherwise carry a
/// thousand copies of it -- and the script fills it in from the card that was
/// clicked. Only the font's own grid and the preview controls are written in
/// here, because both are the same for every icon in the set.
fn detail(recolorable: bool) -> String {
  let ink = if recolorable {
    format!(
      r##"        <div class="stage-row">
          <span class="stage-key">Ink</span>
{swatches}          <label class="picker-ring ring-1 ring-black/10 dark:ring-white/20" title="Choose any ink">
            <input id="detail-picker" class="ink-picker" type="color" value="#2563eb">
          </label>
          <span id="ink-dead" hidden class="stage-hint">fixed by the artwork</span>
        </div>
"##,
      swatches = stage_swatches("color", "swatch", "background:")
    )
  } else {
    String::new()
  };

  format!(
    r##"<dialog id="detail" class="detail" aria-labelledby="detail-name">
  <div class="detail-shell" tabindex="-1" autofocus>

    <div class="stage">
      <div id="canvas" class="canvas" data-guides="on">
        <div class="em-box">
          <span id="detail-glyph" class="stage-glyph" aria-hidden="true"></span>
          <span id="detail-bbox" class="ink-box"></span>
          <span id="bearing-start" class="bearing"></span>
          <span id="bearing-end" class="bearing"></span>
          <span class="rule" style="--at:0%" data-label="ascender"></span>
          <span id="rule-base" class="rule" data-label="baseline"></span>
          <span class="rule" style="--at:100%" data-label="descender"></span>
        </div>
        <span id="detail-advance" class="stage-caption font-mono"></span>
        <button type="button" id="guides" class="stage-toggle font-mono" aria-pressed="true"
                title="Show or hide the metrics">guides</button>
      </div>
      <div class="stage-bar">
{ink}        <div class="stage-row">
          <span class="stage-key">Back</span>
          <button type="button" data-bg="" title="The preview's own background"
                  class="bg-swatch size-5 cursor-pointer rounded-full border border-black/10 dark:border-white/15"></button>
{backdrops}          <label class="picker-ring ring-1 ring-black/10 dark:ring-white/20" title="Choose any background">
            <input id="detail-bg-picker" type="color" value="#ffffff">
          </label>
        </div>
      </div>
    </div>

    <aside class="side">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p id="detail-folder" class="font-mono text-[10px] tracking-[0.22em] text-zinc-400 uppercase dark:text-zinc-500"></p>
          <h2 id="detail-name" class="font-display text-[clamp(1.35rem,3vw,1.85rem)] leading-[1.05] tracking-tight break-all"></h2>
          <p id="detail-meta" class="mt-1.5 font-mono text-[11px] break-all text-zinc-400 dark:text-zinc-500"></p>
        </div>
        <button type="button" id="detail-close" aria-label="Close" title="Close (Esc)"
                class="-mt-1 -mr-1 size-8 shrink-0 cursor-pointer rounded-full text-[13px] leading-none
                       text-zinc-400 transition hover:bg-zinc-100 hover:text-zinc-900
                       dark:hover:bg-zinc-900 dark:hover:text-zinc-100">&#x2715;</button>
      </div>

      <div class="mt-5 flex flex-col gap-1.5">
        <button type="button" id="copy-class" class="row" data-copy="">
          <span class="row-label">Class</span><code class="row-value"></code><span class="row-hint">Copy</span>
        </button>
        <button type="button" id="copy-html" class="row" data-copy="">
          <span class="row-label">HTML</span><code class="row-value"></code><span class="row-hint">Copy</span>
        </button>
        <button type="button" id="copy-css" class="row" data-copy="">
          <span class="row-label">CSS</span><code class="row-value"></code><span class="row-hint">Copy</span>
        </button>
      </div>

      <details class="fold mt-6">
        <summary class="fold-summary font-mono text-[10px] tracking-[0.22em] text-zinc-400 uppercase
                        hover:text-zinc-600 dark:text-zinc-500 dark:hover:text-zinc-400">Metrics</summary>
        <dl id="detail-metrics" class="metrics mt-2"></dl>
        <p id="detail-spill" hidden class="mt-2 text-[12px] text-zinc-500 dark:text-zinc-400"></p>
        <p class="mt-2 font-mono text-[10px] text-zinc-300 dark:text-zinc-600">
          {units} units/em &#183; ascender {ascender} &#183; descender {descender}
        </p>
      </details>

      <div class="mt-6">
        <p class="font-mono text-[10px] tracking-[0.22em] text-zinc-400 uppercase dark:text-zinc-500">Similar names</p>
        <div id="detail-related" class="related-grid mt-2"></div>
        <p id="detail-lonely" hidden class="mt-2 text-[12px] text-zinc-400 dark:text-zinc-500">
          No other icon shares a word with this name.
        </p>
      </div>

      <div class="side-foot">
        <button type="button" id="detail-prev" class="step" aria-label="Previous icon" title="Previous (&#x2190;)">&#x2039;</button>
        <span id="detail-count" class="font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase dark:text-zinc-500"></span>
        <button type="button" id="detail-next" class="step" aria-label="Next icon" title="Next (&#x2192;)">&#x203a;</button>
      </div>
    </aside>

  </div>
</dialog>
"##,
    ink = ink,
    backdrops = stage_swatches("bg", "bg-swatch", "background:"),
    units = UNITS_PER_EM,
    ascender = ASCENDER,
    descender = DESCENDER,
  )
}

/// One row of preset colors for the dialog's stage, smaller than the ones in
/// the bar: the stage has two rows to fit where the bar had one.
fn stage_swatches(attribute: &str, class: &str, style: &str) -> String {
  SWATCHES
    .iter()
    .map(|color| {
      format!(
        "          <button type=\"button\" data-{attribute}=\"{color}\" title=\"{color}\"\n\
         \x20                 class=\"{class} size-5 cursor-pointer rounded-full border \
         border-black/10 dark:border-white/15\" style=\"{style}{color}\"></button>\n"
      )
    })
    .collect()
}

const SCRIPT: &str = r#"  const cards = Array.from(document.querySelectorAll('.card'));
  const groups = Array.from(document.querySelectorAll('.icon-group'));
  const search = document.getElementById('search');
  const shown = document.getElementById('shown');
  const empty = document.getElementById('empty');
  const toast = document.getElementById('toast');

  // The picker sets a custom property on the document that only glyphs read,
  // so the icons follow it while the surrounding text stays readable. It is
  // set on the root rather than on the grid because the detail dialog sits
  // outside the grid and its glyphs have to follow the picker as well. Icons
  // drawn with currentColor pick it up; COLR icons keep the colors baked into
  // the font.
  const grid = document.documentElement;
  // The bar has one set of ink controls and the dialog another, so that the
  // color can be changed without closing the icon being judged. Both drive the
  // same property: there is one ink, wherever it was set from.
  const swatches = Array.from(document.querySelectorAll('.swatch'));
  const pickers = Array.from(document.querySelectorAll('.ink-picker'));
  const colorNote = document.getElementById('color-note');

  const BLACK = '#000000';
  const WHITE = '#ffffff';
  let ink = BLACK;

  // A set with nothing to recolor is rendered without these controls, so
  // everything that touches them has to cope with their absence.
  function applyColor(color) {
    ink = (color || BLACK).toLowerCase();
    grid.style.setProperty('--icon-color', ink);
    if (colorNote) colorNote.textContent = ink;
    for (const picker of pickers) picker.value = ink;
    for (const swatch of swatches) {
      const on = swatch.dataset.color.toLowerCase() === ink;
      swatch.classList.toggle('ring-2', on);
      swatch.classList.toggle('ring-blue-500', on);
      swatch.classList.toggle('ring-offset-1', on);
    }
    try { localStorage.setItem('icofon-color', ink); } catch {}
  }
  for (const swatch of swatches) {
    swatch.addEventListener('click', () => applyColor(swatch.dataset.color));
  }
  for (const picker of pickers) {
    picker.addEventListener('input', () => applyColor(picker.value));
  }
  let storedColor = '';
  try { storedColor = localStorage.getItem('icofon-color') || ''; } catch {}
  applyColor(storedColor);

  // The color the icon is judged against. An icon drawn for a white page and
  // an icon drawn for a dark one are different drawings, and the only way to
  // tell which you have is to put it on the background it is going to live on.
  // Separate from the page theme, which moves the whole preview at once.
  const canvas = document.getElementById('canvas');
  const backdrops = Array.from(document.querySelectorAll('.bg-swatch'));
  const bgPicker = document.getElementById('detail-bg-picker');

  // How the guides are drawn over a dark backdrop and over a light one. The
  // page's own values are left alone: without a backdrop the stage is the page.
  const GUIDES_ON_DARK = [['--rule', 'rgba(250,250,250,.34)'],
                          ['--grid', 'rgba(250,250,250,.08)'],
                          ['--faint', 'rgba(250,250,250,.5)']];
  const GUIDES_ON_LIGHT = [['--rule', 'rgba(9,9,11,.26)'],
                           ['--grid', 'rgba(9,9,11,.07)'],
                           ['--faint', 'rgba(9,9,11,.45)']];

  // Whether text drawn dark would read on this color. sRGB relative luminance,
  // the same measure a contrast ratio is built from.
  function isLight(hex) {
    const packed = parseInt(hex.slice(1), 16);
    if (!Number.isFinite(packed)) return true;
    const channel = (value) => {
      value /= 255;
      return value <= 0.03928 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * channel(packed >> 16 & 255)
         + 0.7152 * channel(packed >> 8 & 255)
         + 0.0722 * channel(packed & 255) > 0.35;
  }

  function applyBackdrop(color) {
    const backdrop = (color || '').toLowerCase();
    // Empty is the preview's own surface, grid and all -- a color under the
    // grid reads as neither the color asked for nor a grid.
    if (backdrop) canvas.dataset.bg = backdrop; else delete canvas.dataset.bg;
    canvas.style.setProperty('--backdrop', backdrop || 'transparent');
    // A guide drawn in the page's ink vanishes on a backdrop chosen to be
    // nothing like the page, so the guides follow the backdrop instead.
    for (const [name, value] of (isLight(backdrop) ? GUIDES_ON_LIGHT : GUIDES_ON_DARK)) {
      if (backdrop) canvas.style.setProperty(name, value);
      else canvas.style.removeProperty(name);
    }
    if (bgPicker && backdrop) bgPicker.value = backdrop;
    for (const swatch of backdrops) {
      const on = swatch.dataset.bg.toLowerCase() === backdrop;
      swatch.classList.toggle('ring-2', on);
      swatch.classList.toggle('ring-blue-500', on);
      swatch.classList.toggle('ring-offset-1', on);
    }
    try { localStorage.setItem('icofon-backdrop', backdrop); } catch {}
  }
  for (const swatch of backdrops) {
    swatch.addEventListener('click', () => applyBackdrop(swatch.dataset.bg));
  }
  if (bgPicker) bgPicker.addEventListener('input', () => applyBackdrop(bgPicker.value));
  let storedBackdrop = '';
  try { storedBackdrop = localStorage.getItem('icofon-backdrop') || ''; } catch {}
  applyBackdrop(storedBackdrop);

  const themeButtons = Array.from(document.querySelectorAll('.theme-btn'));
  function paintTheme() {
    const active = document.documentElement.dataset.theme;
    for (const button of themeButtons) {
      const on = button.dataset.themeSet === active;
      button.classList.toggle('bg-zinc-900', on);
      button.classList.toggle('text-white', on);
      button.classList.toggle('dark:bg-zinc-100', on);
      button.classList.toggle('dark:text-zinc-900', on);
      button.classList.toggle('text-zinc-400', !on);
    }
  }
  for (const button of themeButtons) {
    button.addEventListener('click', () => {
      const theme = button.dataset.themeSet;
      document.documentElement.dataset.theme = theme;
      try { localStorage.setItem('icofon-theme', theme); } catch {}
      // Plain black on a black page is invisible, so the two extremes follow
      // the background. Any other color is left exactly as chosen.
      if (theme === 'dark' && ink === BLACK) applyColor(WHITE);
      if (theme === 'light' && ink === WHITE) applyColor(BLACK);
      paintTheme();
    });
  }
  paintTheme();

  // Search, color bucket and folder all narrow the same list, so they are
  // applied together rather than each owning its own pass.
  const chips = Array.from(document.querySelectorAll('.chip'));
  const folder = document.getElementById('folder');
  let kind = '';

  function paintChips() {
    for (const chip of chips) {
      const on = chip.dataset.kind === kind;
      chip.classList.toggle('border-zinc-900', on);
      chip.classList.toggle('text-zinc-900', on);
      chip.classList.toggle('dark:border-zinc-100', on);
      chip.classList.toggle('dark:text-zinc-100', on);
    }
  }

  function applyFilters() {
    const query = search.value.trim().toLowerCase();
    const wanted = folder ? folder.value : '';
    let visible = 0;
    for (const card of cards) {
      const match =
        (!query || card.dataset.search.includes(query)) &&
        (!kind || card.dataset.kind === kind) &&
        (!wanted || card.dataset.group === wanted);
      card.hidden = !match;
      card.classList.toggle('hidden', !match);
      if (match) visible++;
    }
    // A group whose icons have all been filtered out should take its heading
    // with it, rather than leaving a label over an empty space.
    for (const group of groups) {
      const survives = group.querySelector('.card:not([hidden])') !== null;
      group.hidden = !survives;
      group.classList.toggle('hidden', !survives);
    }
    shown.textContent = visible;
    empty.hidden = visible > 0;
  }

  search.addEventListener('input', applyFilters);
  if (folder) folder.addEventListener('change', applyFilters);
  for (const chip of chips) {
    chip.addEventListener('click', () => {
      // Clicking the active bucket clears it, so the chips act as a filter
      // rather than a mode you cannot leave.
      kind = kind === chip.dataset.kind ? '' : chip.dataset.kind;
      paintChips();
      applyFilters();
    });
  }
  paintChips();

  // A slash jumps to the search box; escape clears whatever is in it. With
  // the dialog open the arrows walk the set instead, and the search box is
  // not on the page to jump to.
  document.addEventListener('keydown', (event) => {
    if (detail.open) {
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault();
        step(event.key === 'ArrowRight' ? 1 : -1);
      } else if (event.key === 'Escape') {
        // Taken here rather than left to the dialog's own escape handling,
        // which would snap it shut without the closing animation.
        event.preventDefault();
        dismiss();
      }
      return;
    }
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName || '');
    if (event.key === '/' && !typing) {
      event.preventDefault();
      search.focus();
      search.select();
    } else if (event.key === 'Escape' && document.activeElement === search) {
      search.value = '';
      applyFilters();
    }
  });

  // The bar only takes on the page background once it has actually stuck, so
  // at rest it reads as part of the page rather than as floating chrome.
  const sentinel = document.getElementById('sentinel');
  const bar = document.getElementById('bar');
  new IntersectionObserver(
    ([entry]) => bar.toggleAttribute('data-stuck', !entry.isIntersecting),
    { threshold: 1 }
  ).observe(sentinel);

  // ---------------------------------------------------------------------
  // The detail dialog.
  //
  // A card can only abbreviate: the glyph is small, the class is elided and
  // nothing says what else in the set is like it. Clicking one opens the icon
  // at size, on its own metrics, with the rest spelled out.

  const detail = document.getElementById('detail');
  const detailGlyph = document.getElementById('detail-glyph');
  const detailName = document.getElementById('detail-name');
  const detailFolder = document.getElementById('detail-folder');
  const detailMeta = document.getElementById('detail-meta');
  const detailAdvance = document.getElementById('detail-advance');
  const detailRelated = document.getElementById('detail-related');
  const detailLonely = document.getElementById('detail-lonely');
  const detailCount = document.getElementById('detail-count');
  const copyClass = document.getElementById('copy-class');
  const copyHtml = document.getElementById('copy-html');
  const copyCss = document.getElementById('copy-css');
  const detailMetrics = document.getElementById('detail-metrics');
  const detailSpill = document.getElementById('detail-spill');
  const detailBbox = document.getElementById('detail-bbox');
  const bearingStart = document.getElementById('bearing-start');
  const bearingEnd = document.getElementById('bearing-end');
  const inkDead = document.getElementById('ink-dead');

  // The guides are placed from the font's own grid rather than from numbers
  // written into the stylesheet, so the baseline is drawn where the baseline
  // is even if the em is ever divided differently.
  document.getElementById('rule-base').style.setProperty(
    '--at', (100 * EM.ascender / (EM.ascender - EM.descender)) + '%');

  // Measurements drawn over the artwork are what you want while judging the
  // metrics and the last thing you want while judging the drawing, so they
  // come off in one click -- and stay off.
  const guides = document.getElementById('guides');
  function applyGuides(on) {
    canvas.dataset.guides = on ? 'on' : 'off';
    guides.setAttribute('aria-pressed', String(on));
    try { localStorage.setItem('icofon-guides', on ? 'on' : 'off'); } catch {}
  }
  guides.addEventListener('click', () => applyGuides(canvas.dataset.guides !== 'on'));
  let storedGuides = '';
  try { storedGuides = localStorage.getItem('icofon-guides') || ''; } catch {}
  applyGuides(storedGuides !== 'off');

  // What the color buckets are called in a sentence. On a card the label is
  // dropped when every icon shares it, since it would then draw no
  // distinction; here one icon is being read on its own, so it always earns
  // its place.
  const KINDS = { single: 'recolorable', mixed: 'partly fixed color', fixed: 'fixed color' };

  // Everything the dialog shows about an icon, taken from the card itself so
  // the page carries one copy of it rather than two.
  function read(card) {
    const name = card.querySelector('.name');
    return {
      card,
      full: card.dataset.name,
      label: card.querySelector('.card-name').textContent,
      group: card.dataset.group,
      code: card.dataset.code,
      attr: name.dataset.copy,
      selector: name.textContent,
      aspect: Number(card.style.getPropertyValue('--aspect')) || 1,
      kind: KINDS[card.dataset.kind] || '',
      file: card.dataset.file || '',
      // A glyph with nothing drawn in it has no bounds to report, and every
      // number taken from them has to give way rather than read as zero.
      bounds: card.dataset.bbox ? card.dataset.bbox.split(' ').map(Number) : null,
      outline: card.dataset.outline.split(' ').map(Number),
    };
  }

  // Font units carry a sign, and a hyphen in a column of measurements reads as
  // a dash between them. The real minus does not.
  const signed = (value) => (value < 0 ? '\u2212' + Math.abs(value) : String(value));
  const plural = (count, thing) => count + ' ' + thing + (count === 1 ? '' : 's');

  // The glyph as the font holds it: what it is given, what it uses of that,
  // and what it costs to draw. Everything is in font units, which the note
  // under the list ties back to the em.
  function metrics(icon) {
    const advance = Math.round(icon.aspect * EM.units);
    const rows = [['Advance', advance + ' u \u00b7 ' + icon.aspect.toFixed(2) + ' em']];
    if (icon.bounds) {
      const [x0, y0, x1, y1] = icon.bounds;
      rows.push(['Ink', (x1 - x0) + ' \u00d7 ' + (y1 - y0) + ' u']);
      rows.push(['Across', signed(x0) + ' \u2192 ' + signed(x1)]);
      rows.push(['Up', signed(y0) + ' \u2192 ' + signed(y1)]);
      rows.push(['Bearings', signed(x0) + ' left \u00b7 ' + signed(advance - x1) + ' right']);
    } else {
      rows.push(['Ink', 'nothing drawn']);
    }
    rows.push(['Outline',
      plural(icon.outline[0], 'contour') + ' \u00b7 ' + plural(icon.outline[1], 'node')]);
    if (icon.file) rows.push(['File', icon.file]);
    return rows;
  }

  // Where the glyph runs past the box the font gave it. Harmless on its own --
  // nothing clips it -- but it is what makes one icon sit taller than its
  // neighbours in a line of text, or touch the next one along.
  function spill(icon) {
    if (!icon.bounds) return '';
    const advance = Math.round(icon.aspect * EM.units);
    const [x0, y0, x1, y1] = icon.bounds;
    const past = [];
    if (y1 > EM.ascender) past.push((y1 - EM.ascender) + ' u above the ascender');
    if (y0 < EM.descender) past.push((EM.descender - y0) + ' u below the descender');
    if (x0 < 0) past.push((-x0) + ' u left of the origin');
    if (x1 > advance) past.push((x1 - advance) + ' u past the advance');
    return past.length ? 'Runs ' + past.join(', ') + '.' : '';
  }

  // Names are matched on the leaf, not on the full name: the folder is
  // already in every name under it, so matching on the whole would make every
  // icon in a folder look like every other one and drown the real kinship.
  // The folder still counts, as a nudge further down.
  const words = (name) => name.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  const spelling = new Map(
    cards.map((card) => [card, words(card.querySelector('.card-name').textContent)]));

  // How much two names have in common. A whole word shared counts most; a
  // longer spelling of the same word -- "arrow" against "arrows" -- still
  // counts, because those name one family and someone looking at the one
  // wants the other.
  function affinity(mine, theirs) {
    let score = 0;
    for (const word of mine) {
      for (const other of theirs) {
        if (word === other) { score += 3; break; }
        const long = word.length > 3 && other.length > 3;
        if (long && (word.startsWith(other) || other.startsWith(word))) { score += 2; break; }
      }
    }
    return score;
  }

  // The icons closest to this one by name. Filters are deliberately ignored:
  // a search narrow enough to be worth opening an icon from is usually narrow
  // enough to have hidden everything it is like.
  function similar(icon) {
    const mine = spelling.get(icon.card);
    const matches = [];
    for (const card of cards) {
      if (card === icon.card) continue;
      const theirs = spelling.get(card);
      let score = affinity(mine, theirs);
      if (!score) continue;
      // A shared first word is what makes a family -- arrow-left beside
      // arrow-right -- so it outranks the same word turning up anywhere.
      if (theirs[0] === mine[0]) score += 2;
      if (card.dataset.group === icon.group) score += 1;
      matches.push({ card, score });
    }
    matches.sort((a, b) =>
      b.score - a.score || a.card.dataset.name.localeCompare(b.card.dataset.name));
    return matches.slice(0, 12).map((match) => match.card);
  }

  // Which list the arrows walk. With a filter on they follow what is left on
  // the page; an icon reached through the similar list can be one the filter
  // hides, and then the whole set is what there is to walk.
  function neighbours() {
    const visible = cards.filter((card) => !card.hidden);
    return visible.includes(current) ? visible : cards;
  }

  let current = null;

  function show(card) {
    current = card;
    const icon = read(card);

    detailGlyph.className = 'stage-glyph ' + icon.attr;
    detailGlyph.style.setProperty('--aspect', icon.aspect);
    detailName.textContent = icon.label;
    detailFolder.textContent = icon.group;
    detailFolder.hidden = !icon.group;
    detailMeta.textContent =
      ['U+' + icon.code.toUpperCase(), icon.kind].filter(Boolean).join(' · ');
    detailAdvance.textContent = icon.aspect.toFixed(2) + ' em advance';

    // The numbers, and the same numbers drawn on the glyph: the ink's own box
    // inside the em box, and the side bearings as the gaps either side of it.
    detailMetrics.replaceChildren(...metrics(icon).flatMap(([key, value]) => {
      const term = document.createElement('dt');
      term.textContent = key;
      const detail = document.createElement('dd');
      detail.textContent = value;
      return [term, detail];
    }));
    const past = spill(icon);
    detailSpill.textContent = past;
    detailSpill.hidden = !past;

    detailBbox.hidden = !icon.bounds;
    bearingStart.hidden = !icon.bounds;
    bearingEnd.hidden = !icon.bounds;
    if (icon.bounds) {
      const advance = Math.round(icon.aspect * EM.units);
      const [x0, y0, x1, y1] = icon.bounds;
      const across = (units) => (100 * units / advance) + '%';
      const up = (units) => (100 * units / (EM.ascender - EM.descender)) + '%';
      detailBbox.style.left = across(x0);
      detailBbox.style.right = across(advance - x1);
      detailBbox.style.top = up(EM.ascender - y1);
      detailBbox.style.bottom = up(y0 - EM.descender);
      bearingStart.style.width = across(Math.max(x0, 0));
      bearingEnd.style.width = across(Math.max(advance - x1, 0));
    }

    // The ink controls move nothing on an icon whose colors are all the
    // artwork's, so they say so rather than looking broken.
    if (inkDead) inkDead.hidden = icon.kind !== KINDS.fixed;

    // The selector is what a stylesheet matches on and the attribute is what
    // a class attribute takes, so each row shows the one and copies the other.
    copyClass.querySelector('.row-value').textContent = icon.selector;
    copyClass.dataset.copy = icon.attr;
    const markup = '<span class="' + icon.attr + '"></span>';
    copyHtml.querySelector('.row-value').textContent = markup;
    copyHtml.dataset.copy = markup;
    const content = 'content: "\\' + icon.code + '";';
    copyCss.querySelector('.row-value').textContent = content;
    copyCss.dataset.copy = content;

    const matches = similar(icon);
    detailLonely.hidden = matches.length > 0;
    detailRelated.replaceChildren(...matches.map((match, index) => {
      const near = read(match);
      const tile = document.createElement('button');
      tile.type = 'button';
      tile.className = 'related';
      tile.title = near.full;
      tile.style.setProperty('--aspect', near.aspect);
      tile.style.setProperty('--i', index);
      const glyph = document.createElement('span');
      glyph.className = 'glyph ' + near.attr;
      glyph.setAttribute('aria-hidden', 'true');
      const label = document.createElement('span');
      label.className = 'related-name';
      label.textContent = near.label;
      tile.append(glyph, label);
      tile.addEventListener('click', () => show(match));
      return tile;
    }));

    const list = neighbours();
    detailCount.textContent = (list.indexOf(current) + 1) + ' / ' + list.length;
    // A tall similar list left scrolled down would hide the icon that was
    // just asked for.
    detail.querySelector('.side').scrollTop = 0;
  }

  function step(delta) {
    const list = neighbours();
    const at = list.indexOf(current);
    show(list[(at + delta + list.length) % list.length]);
  }

  // Closing is animated, so the dialog is left open until the animation ends.
  // Only the dialog itself carries that animation, which is what the target
  // check is reading.
  function dismiss() {
    if (detail.open) detail.dataset.closing = '';
  }
  detail.addEventListener('animationend', (event) => {
    if (event.target === detail && 'closing' in detail.dataset) {
      delete detail.dataset.closing;
      detail.close();
    }
  });

  document.addEventListener('click', (event) => {
    // Copying the class is not also a request to open the icon, so a click
    // that lands on a copy button stops there.
    if (event.target.closest('[data-copy]')) return;
    const card = event.target.closest('.card');
    if (!card || detail.open) return;
    show(card);
    detail.showModal();
  });

  document.getElementById('detail-close').addEventListener('click', dismiss);
  document.getElementById('detail-prev').addEventListener('click', () => step(-1));
  document.getElementById('detail-next').addEventListener('click', () => step(1));
  // The dialog is exactly its own box, so a click that lands on the element
  // itself came from the backdrop around it.
  detail.addEventListener('click', (event) => {
    if (event.target === detail) dismiss();
  });
  // Escape closes it the same way the button does, rather than snapping shut.
  detail.addEventListener('cancel', (event) => {
    event.preventDefault();
    dismiss();
  });

  let fading;
  let hiding;
  document.addEventListener('click', async (event) => {
    const button = event.target.closest('[data-copy]');
    if (!button) return;
    const text = button.dataset.copy;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // The async clipboard needs a secure context, which a page opened
      // straight off the filesystem is not. Fall back to a selection copy.
      const field = document.createElement('textarea');
      field.value = text;
      field.style.position = 'fixed';
      field.style.opacity = '0';
      document.body.append(field);
      field.select();
      document.execCommand('copy');
      field.remove();
    }
    toast.textContent = 'Copied ' + text;
    // A modal dialog is in the top layer, where an ordinary fixed element
    // cannot reach however it is stacked. A popover enters the same layer,
    // and later than the dialog did, so the toast lands in front of it.
    if (!toast.matches(':popover-open')) toast.showPopover();
    toast.classList.remove('opacity-0', 'translate-y-3');
    clearTimeout(fading);
    clearTimeout(hiding);
    fading = setTimeout(() => toast.classList.add('opacity-0', 'translate-y-3'), 1400);
    // Left in the layer until the fade is over, so it is not cut off mid-way.
    hiding = setTimeout(() => toast.hidePopover(), 1700);
  });"#;

/// Standard base64, which is how an SVG is spelled inside a `data:` URI.
///
/// Three bytes make four characters; a short last group is padded with `=` so
/// the length stays a multiple of four.
fn base64(data: &[u8]) -> String {
  const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

  let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
  for group in data.chunks(3) {
    let bits = group.iter().enumerate().fold(0u32, |bits, (index, byte)| {
      bits | u32::from(*byte) << (16 - 8 * index)
    });
    for slot in 0..4 {
      // A group of n bytes fills n + 1 slots; the rest are padding.
      if slot <= group.len() {
        out.push(ALPHABET[(bits >> (18 - 6 * slot) & 0x3F) as usize] as char);
      } else {
        out.push('=');
      }
    }
  }
  out
}

fn escape(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for ch in value.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      '\'' => out.push_str("&#39;"),
      _ => out.push(ch),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::svg;

  /// The default naming: a single prefixed class per icon.
  fn classes(prefix: &str) -> Classes<'_> {
    Classes {
      prefix,
      base_class: false,
    }
  }

  /// The page down to the detail dialog: the grid, its headings and its
  /// cards. The dialog and the script that fills it are the same on every
  /// page and carry words -- a heading, the name of a color bucket -- that a
  /// test about the grid should not be counting.
  fn grid(page: &str) -> &str {
    page.split("<dialog").next().unwrap()
  }

  fn icon(name: &str, codepoint: char) -> Icon {
    grouped(name, codepoint, None)
  }

  /// Builds an icon the way `load_icons` does: the group is folded into the
  /// name, and the label is the file part that is left.
  fn grouped(label: &str, codepoint: char, group: Option<&str>) -> Icon {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                        <rect width="24" height="24" fill="currentColor"/>
                      </svg>"##;
    let name = match group {
      Some(group) => format!("{}-{label}", group.replace('/', "-")),
      None => label.to_string(),
    };
    Icon {
      name,
      label: label.to_string(),
      group: group.map(str::to_string),
      source: label.into(),
      codepoint,
      outline: svg::parse(svg.as_bytes(), label, crate::config::Color::Keep).unwrap(),
    }
  }

  #[test]
  fn a_base_class_is_written_on_the_glyph_and_offered_for_copying() {
    let classes = Classes {
      prefix: "icon",
      base_class: true,
    };
    let page = render(&[icon("arrow-left", '\u{e900}')], "Icons", classes, "f.css");

    assert!(page.contains(r#"<span class="glyph icon icon-arrow-left "#));
    // The button copies what goes in a `class` attribute and shows the
    // selector that matches it.
    assert!(page.contains(r#"data-copy="icon icon-arrow-left""#));
    assert!(page.contains(">.icon.icon-arrow-left</button>"));
  }

  #[test]
  fn only_icon_sections_carry_the_class_the_filter_hides() {
    // Filtering hides every `.icon-group` left without a visible card. The
    // section hook used to be `group`, which is also a Tailwind utility, and
    // the search box's own label carries it -- so typing a query hid the
    // search box. The hook must stay unique to sections.
    let page = render(
      &[grouped("left", '\u{e900}', Some("arrows"))],
      "Icons",
      classes("icon"),
      "f.css",
    );

    assert_eq!(page.matches(r#"class="icon-group"#).count(), 1);
    assert!(page.contains(r#"<section class="icon-group mb-8" data-group="arrows">"#));
  }

  #[test]
  fn lists_every_icon_with_its_class_and_codepoint() {
    let icons = vec![icon("arrow-left", '\u{e900}'), icon("star", '\u{e9f0}')];
    let page = render(&icons, "My Icons", classes("ico"), "icons.css");

    assert!(page.contains(r#"<link rel="stylesheet" href="icons.css">"#));
    // Each card carries the glyph, the bare name, the class and the codepoint.
    assert!(page.contains(r#"<span class="glyph ico-arrow-left "#));
    assert!(page.contains(">arrow-left</span>"));
    assert!(page.contains(r#"data-copy="ico-arrow-left""#));
    assert!(page.contains(">.ico-arrow-left</button>"));
    assert!(page.contains("U+E900"));
    assert!(page.contains(r#"<span class="glyph ico-star "#));
    assert!(page.contains("U+E9F0"));
    assert_eq!(page.matches("data-search=").count(), icons.len());
  }

  #[test]
  fn the_header_offers_a_color_palette() {
    let page = render(
      &[icon("plain", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    for swatch in SWATCHES {
      assert!(
        page.contains(&format!(r#"data-color="{swatch}""#)),
        "{swatch}"
      );
    }
    // Black and white lead, so both backgrounds are one click away.
    assert!(page.contains(r##"data-color="#000000""##));
    assert!(page.contains(r##"data-color="#ffffff""##));
    // Every swatch is a real color: there is no "inherit" option to leave
    // the readout blank.
    assert!(!page.contains(r#"data-color="""#));
    assert!(page.contains(r#"id="picker" class="ink-picker" type="color""#));
    // The dialog carries its own ink and background controls, so an icon can
    // be tried on a color without closing the one being looked at.
    assert!(page.contains(r#"id="detail-picker" class="ink-picker" type="color""#));
    assert!(page.contains(r#"id="detail-bg-picker" type="color""#));
    assert!(page.contains(r##"data-bg="#000000""##));
    // The one background that is not a color: the preview's own surface.
    assert!(page.contains(r#"data-bg="""#));
  }

  #[test]
  fn a_card_carries_the_glyph_metrics_the_dialog_reports() {
    // The dialog is filled from the card that was clicked, so everything it
    // has to say about a glyph must already be on the card.
    let page = render(
      &[icon("square", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    // The icon is one rect drawn edge to edge in its viewBox, so it fills the
    // em box exactly: the full advance across, ascender to descender up.
    assert!(page.contains(r#"data-bbox="0 -200 1000 800""#), "{page}");
    assert!(page.contains(r#"data-outline="1 4""#), "{page}");
    assert!(page.contains(r#"data-file="square""#), "{page}");
  }

  #[test]
  fn a_blank_glyph_reports_no_bounds_rather_than_a_point_at_the_origin() {
    // Nothing drawn is not the same as something drawn at zero size, and the
    // metrics have to say so rather than reporting a box with no width.
    let mut blank = icon("blank", '\u{e900}');
    blank.outline.path = kurbo::BezPath::new();
    let page = render(&[blank], "Icons", classes("icon"), "f.css");
    assert!(page.contains(r#"data-bbox="" "#), "{page}");
    assert!(page.contains(r#"data-outline="0 0""#), "{page}");
  }

  #[test]
  fn mixed_and_fixed_icons_are_marked_apart() {
    // A mixed icon still has a part that follows CSS color, so it must not
    // read the same as one that is fixed throughout. The note is only drawn
    // where it draws a distinction, so the two have to be on one page.
    let mut mixed = icon("badge", '\u{e900}');
    mixed.outline.coloring = Coloring::Mixed;
    let mut fixed = icon("logo", '\u{e901}');
    fixed.outline.coloring = Coloring::Fixed;

    let page = render(&[mixed, fixed], "Icons", classes("icon"), "f.css");
    assert!(page.contains(">partly fixed</code>"));
    assert!(page.contains(">fixed</code>"));
    assert!(
      page.contains("partly fixed mixed"),
      "searchable by its kind"
    );
    assert!(page.contains("fixed color multicolor"));
  }

  #[test]
  fn a_set_of_one_kind_drops_the_filter_and_says_so_once() {
    // Chips that can only show everything or nothing are not a filter, and a
    // note on every card draws no distinction. One line above the grid does.
    let mut icons = [icon("logo", '\u{e900}'), icon("badge", '\u{e901}')];
    for icon in &mut icons {
      icon.outline.coloring = Coloring::Fixed;
    }
    let page = render(&icons, "Icons", classes("icon"), "f.css");

    assert!(!page.contains(r#"id="kinds""#), "no chips to click");
    assert!(!page.contains(">fixed</code>"), "no note on the cards");
    assert!(page.contains("color fixed by the artwork"), "said once");
  }

  #[test]
  fn a_set_with_nothing_to_recolor_drops_the_ink_controls() {
    // The palette reaches an icon through a part painted `currentColor`. With
    // no such part it would move a ring around and change nothing.
    let mut fixed = icon("logo", '\u{e900}');
    fixed.outline.coloring = Coloring::Fixed;
    let page = render(&[fixed], "Icons", classes("icon"), "f.css");
    assert!(!page.contains(r#"id="picker""#));
    assert!(!page.contains(r#"id="swatches""#));
    assert!(!page.contains(">Ink</span>"));

    // One recolorable icon is enough to make them worth showing again.
    let mut mixed = icon("badge", '\u{e901}');
    mixed.outline.coloring = Coloring::Mixed;
    let page = render(&[mixed], "Icons", classes("icon"), "f.css");
    assert!(page.contains(r#"id="picker""#));
  }

  #[test]
  fn one_folder_holding_everything_is_not_a_choice() {
    let all_in_one = [
      grouped("left", '\u{e900}', Some("arrows")),
      grouped("right", '\u{e901}', Some("arrows")),
    ];
    assert!(!render(&all_in_one, "Icons", classes("icon"), "f.css").contains(r#"id="folder""#));

    // A folder alongside loose icons is a choice: picking it hides the rest.
    let some_loose = [
      icon("check", '\u{e900}'),
      grouped("left", '\u{e901}', Some("arrows")),
    ];
    assert!(render(&some_loose, "Icons", classes("icon"), "f.css").contains(r#"id="folder""#));
  }

  #[test]
  fn a_one_color_icon_carries_no_warning() {
    // It behaves like every other glyph, so there is nothing to say.
    let page = render(
      &[icon("plain", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(!page.contains(">partly fixed</code>"));
    assert!(!page.contains(">fixed</code>"));
  }

  #[test]
  fn wide_icons_carry_their_aspect_so_the_card_can_shrink_them() {
    let mut wide = icon("wordmark", '\u{e900}');
    wide.outline.advance = 16_538;
    let page = render(&[wide], "Icons", classes("icon"), "f.css");
    assert!(
      page.contains("--aspect:16.538"),
      "the card is told how wide it is"
    );
    assert!(
      page.contains("16.5× wide"),
      "and says so, since it renders small"
    );
  }

  #[test]
  fn ordinary_icons_get_no_width_note() {
    let page = render(
      &[icon("square", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(page.contains("--aspect:1.000"));
    assert!(!page.contains("× wide"));
  }

  #[test]
  fn base64_matches_the_reference_vectors() {
    // RFC 4648's own examples, which cover all three padding cases.
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foob"), "Zm9vYg==");
    assert_eq!(base64(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    // Every byte, so the alphabet and the bit shuffling are both exercised.
    let all: Vec<u8> = (0..=255u8).collect();
    assert_eq!(base64(&all).len(), 344);
    assert!(base64(&all).starts_with("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"));
  }

  #[test]
  fn the_favicon_is_carried_in_the_page() {
    // A preview is one file plus its font; a favicon fetched from beside it
    // would be a fourth thing to copy and the first one to go missing.
    let page = render(
      &[icon("plain", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    let mark = base64(MARK_SVG.as_bytes());
    assert!(page.contains(&format!(
      r#"<link rel="icon" href="data:image/svg+xml;base64,{mark}">"#
    )));
  }

  #[test]
  fn the_byline_carries_the_logo_inline() {
    let page = render(
      &[icon("plain", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(page.contains(HOMEPAGE), "the byline links to the project");
    // Written into the page, not linked: `currentColor` in the wordmark only
    // resolves against the surrounding text when the SVG is part of the
    // document, which is what carries it through the light/dark switch.
    assert!(page.contains(r#"<tspan fill="currentColor">ico</tspan>"#));
  }

  #[test]
  fn pulls_tailwind_from_the_cdn_so_the_page_needs_no_build() {
    let page = render(&[icon("ok", '\u{e900}')], "Icons", classes("icon"), "f.css");
    assert!(page.contains(&format!(r#"<script src="{TAILWIND_CDN}"></script>"#)));
  }

  #[test]
  fn search_index_covers_name_class_and_code() {
    let page = render(
      &[icon("arrow-left", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(page.contains(r#"data-search="arrow-left icon-arrow-left e900""#));
  }

  #[test]
  fn a_grouped_card_shows_the_leaf_name_and_the_full_class() {
    // The heading already says "arrows", so repeating it in the title would
    // just be noise; the class still carries the complete name.
    let page = render(
      &[grouped("left", '\u{e901}', Some("arrows"))],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(page.contains(">left</span>"), "card title is the leaf");
    assert!(page.contains(">.icon-arrows-left</button>"));
    assert!(page.contains(r#"data-copy="icon-arrows-left""#));
  }

  #[test]
  fn subfolders_become_labeled_sections() {
    let icons = vec![
      icon("loose", '\u{e900}'),
      grouped("left", '\u{e901}', Some("arrows")),
      grouped("right", '\u{e902}', Some("arrows")),
      grouped("share", '\u{e903}', Some("social")),
    ];
    let page = render(&icons, "Icons", classes("icon"), "f.css");

    // One section per group, plus the unlabeled one for top-level icons.
    assert_eq!(page.matches("<section").count(), 3);
    assert!(page.contains(r#"data-group="arrows""#));
    assert!(page.contains(r#"data-group="social""#));
    assert!(page.contains(">arrows</h2>"));
    assert!(page.contains(">social</h2>"));
    // Top-level icons sit in a section with no heading.
    assert!(page.contains(r#"data-group=""#));
    assert_eq!(grid(&page).matches("<h2").count(), 2);
  }

  #[test]
  fn a_flat_folder_still_renders_one_unlabeled_section() {
    let page = render(
      &[icon("only", '\u{e900}')],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert_eq!(page.matches("<section").count(), 1);
    assert!(!grid(&page).contains("<h2"));
  }

  #[test]
  fn the_group_is_searchable() {
    let page = render(
      &[grouped("left", '\u{e900}', Some("arrows"))],
      "Icons",
      classes("icon"),
      "f.css",
    );
    assert!(page.contains(r#"data-search="arrows-left icon-arrows-left e900 arrows""#));
  }

  #[test]
  fn a_card_carries_what_the_detail_dialog_reads() {
    // The dialog is one empty shell filled in from the card that was clicked,
    // so the card has to carry the icon's full name and codepoint, and the
    // glyph has to be something to click.
    let page = render(
      &[grouped("left", '\u{e901}', Some("arrows"))],
      "Icons",
      classes("icon"),
      "f.css",
    );

    assert!(page.contains(r#"data-name="arrows-left""#));
    assert!(page.contains(r#"data-code="e901""#));
    assert!(page.contains(r#"aria-label="Open left""#));
    // The leaf name is picked out for matching names against each other.
    assert!(page.contains(r#"class="card-name"#));
  }

  #[test]
  fn the_page_carries_one_empty_detail_dialog() {
    // Not one per icon: the markup is the same whichever card is open, and a
    // large set would otherwise carry a copy of it for every glyph.
    let icons = vec![icon("one", '\u{e900}'), icon("two", '\u{e901}')];
    let page = render(&icons, "Icons", classes("icon"), "f.css");

    assert_eq!(page.matches("<dialog").count(), 1);
    assert!(page.contains(r#"id="detail-glyph""#));
    assert!(page.contains(r#"id="detail-related""#));
    // Class, markup and codepoint are each offered on their own row.
    assert!(page.contains(r#"id="copy-class""#));
    assert!(page.contains(r#"id="copy-html""#));
    assert!(page.contains(r#"id="copy-css""#));
  }

  #[test]
  fn the_toast_can_reach_above_the_dialog() {
    // A modal dialog is in the top layer, which an ordinary fixed element
    // cannot reach -- so a copy made from the dialog would have nothing to
    // show for itself.
    let page = render(&[icon("ok", '\u{e900}')], "Icons", classes("icon"), "f.css");
    assert!(page.contains(r#"<div id="toast" popover="manual""#));
  }

  #[test]
  fn markup_from_user_supplied_names_is_escaped() {
    let page = render(
      &[icon("ok", '\u{e900}')],
      "<script>alert(1)</script>",
      classes("icon"),
      "f.css",
    );
    assert!(!page.contains("<script>alert(1)</script>"));
    assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
  }
}
