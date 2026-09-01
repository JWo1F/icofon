//! A browsable preview page listing every icon in the generated font.

use crate::css::Classes;
use crate::font::Icon;
use crate::svg::Coloring;

/// Tailwind's browser build compiles utility classes at runtime, so the preview
/// page needs no build step of its own.
const TAILWIND_CDN: &str = "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4";

/// Render `example.html`: a searchable grid of every icon, showing its glyph,
/// name, CSS class and codepoint.
///
/// `css_url` is the stylesheet's location relative to the page, so the preview
/// exercises exactly the same CSS a site would use.
pub fn render(icons: &[Icon], family: &str, classes: Classes<'_>, css_url: &str) -> String {
  let family = escape(family);
  let mut page = String::new();

  page.push_str(&format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{family} — icons</title>
<link rel="stylesheet" href="{css}">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Instrument+Serif:ital@0;1&family=IBM+Plex+Mono:wght@400;500&display=swap">
<script src="{tailwind}"></script>
<style type="text/tailwindcss">
  /* Drive dark mode from the toggle rather than the OS, so an icon can be
     checked on both backgrounds without changing system settings. */
  @custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));
  @theme {{
    --font-mono: "IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
    --font-display: "Instrument Serif", ui-serif, Georgia, "Times New Roman", serif;
  }}
</style>
<style>
  :root {{ --bar: #ffffff; --hair: rgba(9,9,11,.10); }}
  [data-theme="dark"] {{ --bar: #09090b; --hair: rgba(250,250,250,.14); }}

  /* An icon is one em tall but may be many ems wide, so each card is measured
     and the glyph is scaled down by its own width to fit. Square icons are
     unaffected: their --aspect is 1. */
  .card {{ container-type: inline-size; }}
  .glyph {{
    font-size: min(2.25rem, calc((100cqw - 1.5rem) / var(--aspect, 1)));
    /* Only the glyphs follow the color picker; labels stay readable. */
    color: var(--icon-color, inherit);
  }}

  /* The bar must not change height when it sticks. It is in normal flow, so a
     height change shifts everything below it, the browser's scroll anchoring
     compensates by moving the scroll position, and that flips the sentinel
     back — a loop that reads as the page fighting you on small scrolls. Only
     the background and the mini title change here, neither of which reflows. */
  #bar {{ transition: background-color .25s ease; }}
  #bar[data-stuck] {{
    background: color-mix(in srgb, var(--bar) 88%, transparent);
    backdrop-filter: saturate(1.4) blur(14px);
  }}
  /* Revealed by width, which cannot change the bar's height. */
  #mini {{
    max-width: 0; opacity: 0; overflow: hidden; white-space: nowrap;
    /* The row gap still applies around a zero-width item, which would indent
       the search box away from the labels below it. */
    margin-right: -1.25rem;
    transition: max-width .3s ease, opacity .2s ease, margin .3s ease;
  }}
  #bar[data-stuck] #mini {{ max-width: 14rem; opacity: 1; margin-right: .25rem; }}

  /* A hairline that fades out at both ends, so the bar sits on the page
     instead of being boxed in by it. */
  .hairline {{ height: 1px; background: linear-gradient(90deg, transparent, var(--hair) 12%, var(--hair) 88%, transparent); }}

  /* A color input is a bordered square by default. It is made round, and then
     ringed in spectrum so it reads as "choose any color" rather than as one
     more preset to pick from. */
  #picker {{
    -webkit-appearance: none; appearance: none;
    padding: 0; border: none; background: none;
    border-radius: 9999px; overflow: hidden;
    width: 100%; height: 100%; display: block;
  }}
  #picker::-webkit-color-swatch-wrapper {{ padding: 0; }}
  #picker::-webkit-color-swatch {{ border: none; border-radius: 9999px; }}
  #picker::-moz-color-swatch {{ border: none; border-radius: 9999px; }}
  .picker-ring {{
    background: conic-gradient(from 200deg, #ef4444, #f59e0b, #facc15, #22c55e, #06b6d4, #3b82f6, #a855f7, #ef4444);
    padding: 2px; border-radius: 9999px; width: 1.6rem; height: 1.6rem;
    display: grid; place-items: center; cursor: pointer;
    transition: transform .15s ease;
  }}
  .picker-ring:hover {{ transform: scale(1.12); }}
  .swatch {{ transition: transform .15s ease; }}
  .swatch:hover {{ transform: scale(1.15); }}
</style>
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
      <span id="shown">{total}</span> <span class="text-zinc-300 dark:text-zinc-600">/</span> {total} glyphs
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

      <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-2">
        <span class="font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase dark:text-zinc-500">Ink</span>
        <div id="swatches" class="flex flex-wrap items-center gap-1.5">
{swatches}        </div>
        <span class="mx-1 h-4 w-px bg-zinc-200 dark:bg-zinc-800"></span>
        <label class="picker-ring ring-1 ring-black/10 dark:ring-white/20" title="Choose any color">
          <input id="picker" type="color" value="#2563eb">
        </label>
        <span id="color-note" class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500"></span>
      </div>
    </div>

    <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2">
      <div class="flex items-center gap-2">
        <span class="font-mono text-[10px] tracking-[0.18em] text-zinc-400 uppercase dark:text-zinc-500">Color</span>
        <div id="kinds" class="flex flex-wrap items-center gap-1">
{kinds}        </div>
      </div>
{folders}    </div>

  </div>
  <div class="hairline"></div>
</header>

<main class="pt-7">
"##,
        family = family,
        css = escape(css_url),
        tailwind = TAILWIND_CDN,
        total = icons.len(),
        kinds = kind_chips(icons),
        folders = folder_chips(icons),
        swatches = SWATCHES
            .iter()
            .map(|color| format!(
                "      <button type=\"button\" data-color=\"{color}\" title=\"{color}\"\n\
                 \x20             class=\"swatch size-6 cursor-pointer rounded-full border \
                 border-black/10 dark:border-white/15\" style=\"background:{color}\"></button>\n"
            ))
            .collect::<String>(),
    ));

  // Icons arrive sorted by group, so consecutive runs share a section.
  for (group, members) in group_runs(icons) {
    page.push_str(&section_open(group));
    for icon in members {
      page.push_str(&card(icon, classes));
    }
    page.push_str("  </div>\n</section>\n");
  }

  page.push_str(&format!(
    r#"</main>

<p id="empty" hidden class="mt-10 text-center text-zinc-500 dark:text-zinc-400">
  No icons match that search.
</p>

<div id="toast" role="status" aria-live="polite"
     class="pointer-events-none fixed bottom-7 left-1/2 -translate-x-1/2 translate-y-3 rounded-lg
            bg-zinc-900 px-3.5 py-2 text-sm text-white opacity-0 transition
            dark:bg-zinc-100 dark:text-zinc-900"></div>

</div>
<script>
{script}
</script>
</body>
</html>
"#,
    script = SCRIPT,
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

fn kind_chips(icons: &[Icon]) -> String {
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

fn folder_chips(icons: &[Icon]) -> String {
  let mut folders: Vec<&str> = icons.iter().filter_map(|i| i.group.as_deref()).collect();
  folders.sort_unstable();
  folders.dedup();
  if folders.is_empty() {
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
    r#"<section class="group mb-8" data-group="{group}">
{heading}  <div class="grid grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] gap-3">
"#,
    group = escape(group.unwrap_or_default()),
  )
}

fn note(text: &str) -> String {
  format!(
    r#"
        <code class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500">{text}</code>"#
  )
}

fn card(icon: &Icon, classes: Classes<'_>) -> String {
  // What you would write in a `class` attribute, which is what the button
  // copies, and the selector that matches it, which is what it displays.
  let class = classes.attr(&icon.name);
  let selector = classes.selector(&icon.name);
  let code = icon.codepoint as u32;
  // How many ems wide the glyph is. A wide icon would otherwise run straight
  // out of its card, so the stylesheet divides the display size by this.
  let aspect = f64::from(icon.outline.advance) / f64::from(crate::font::UNITS_PER_EM);
  // Say when an icon will not simply follow the CSS `color`, and how far that
  // goes: a mixed icon still has one part that does.
  let color_note = match icon.outline.coloring {
    Coloring::Single => String::new(),
    Coloring::Mixed => note("partly fixed"),
    Coloring::Fixed => note("fixed"),
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
    r#"    <figure data-search="{search}" data-kind="{kind}" data-group="{group}" style="--aspect:{aspect:.3}"
            class="card m-0 flex flex-col items-center gap-3 rounded-xl border border-zinc-200
                   px-3 pt-5 pb-3.5 text-center dark:border-zinc-800">
      <span class="glyph {class} leading-none" aria-hidden="true"></span>
      <figcaption class="flex w-full flex-col items-center gap-1">
        <span class="max-w-full text-[13px] font-medium break-all">{name}</span>{width_note}{color_note}
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
    code = code,
    aspect = aspect,
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

const SCRIPT: &str = r#"  const cards = Array.from(document.querySelectorAll('.card'));
  const groups = Array.from(document.querySelectorAll('.group'));
  const search = document.getElementById('search');
  const shown = document.getElementById('shown');
  const empty = document.getElementById('empty');
  const toast = document.getElementById('toast');

  // The picker sets a custom property on <main> that only .glyph reads, so the
  // icons follow it while the card text stays readable. Icons drawn with
  // currentColor pick it up; COLR icons keep the colors baked into the font.
  const grid = document.querySelector('main');
  const swatches = Array.from(document.querySelectorAll('.swatch'));
  const picker = document.getElementById('picker');
  const colorNote = document.getElementById('color-note');

  const BLACK = '#000000';
  const WHITE = '#ffffff';
  let ink = BLACK;

  function applyColor(color) {
    ink = (color || BLACK).toLowerCase();
    grid.style.setProperty('--icon-color', ink);
    colorNote.textContent = ink;
    picker.value = ink;
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
  picker.addEventListener('input', () => applyColor(picker.value));
  let storedColor = '';
  try { storedColor = localStorage.getItem('icofon-color') || ''; } catch {}
  applyColor(storedColor);

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

  // A slash jumps to the search box; escape clears whatever is in it.
  document.addEventListener('keydown', (event) => {
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

  let timer;
  document.addEventListener('click', async (event) => {
    const button = event.target.closest('.name');
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
    toast.classList.remove('opacity-0', 'translate-y-3');
    clearTimeout(timer);
    timer = setTimeout(() => toast.classList.add('opacity-0', 'translate-y-3'), 1400);
  });"#;

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

  fn icon(name: &str, codepoint: char) -> Icon {
    grouped(name, codepoint, None)
  }

  /// Builds an icon the way `load_icons` does: the group is folded into the
  /// name, and the label is the file part that is left.
  fn grouped(label: &str, codepoint: char, group: Option<&str>) -> Icon {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                        <rect width="24" height="24" fill="#000"/>
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
      outline: svg::parse(svg.as_bytes(), label).unwrap(),
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
    assert!(page.contains(r#"id="picker" type="color""#));
  }

  #[test]
  fn mixed_and_fixed_icons_are_marked_apart() {
    // A mixed icon still has a part that follows CSS color, so it must not
    // read the same as one that is fixed throughout.
    let mut mixed = icon("badge", '\u{e900}');
    mixed.outline.coloring = Coloring::Mixed;
    let page = render(&[mixed], "Icons", classes("icon"), "f.css");
    assert!(page.contains("partly fixed"));
    assert!(
      page.contains("partly fixed mixed"),
      "searchable by its kind"
    );

    let mut fixed = icon("logo", '\u{e900}');
    fixed.outline.coloring = Coloring::Fixed;
    let page = render(&[fixed], "Icons", classes("icon"), "f.css");
    assert!(page.contains(">fixed</code>"));
    assert!(page.contains("fixed color multicolor"));
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
    assert!(!page.contains("partly fixed"));
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
    assert_eq!(page.matches("<h2").count(), 2);
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
    assert!(!page.contains("<h2"));
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
