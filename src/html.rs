//! A browsable preview page listing every icon in the generated font.

use crate::font::Icon;

/// Tailwind's browser build compiles utility classes at runtime, so the preview
/// page needs no build step of its own.
const TAILWIND_CDN: &str = "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4";

/// Render `example.html`: a searchable grid of every icon, showing its glyph,
/// name, CSS class and codepoint.
///
/// `css_url` is the stylesheet's location relative to the page, so the preview
/// exercises exactly the same CSS a site would use.
pub fn render(icons: &[Icon], family: &str, prefix: &str, css_url: &str) -> String {
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
<script src="{tailwind}"></script>
<style type="text/tailwindcss">
  /* Drive dark mode from the toggle rather than the OS, so an icon can be
     checked on both backgrounds without changing system settings. */
  @custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));
</style>
<style>
  /* An icon is one em tall but may be many ems wide, so each card is measured
     and the glyph is scaled down by its own width to fit. Square icons are
     unaffected: their --aspect is 1. */
  .card {{ container-type: inline-size; }}
  .glyph {{
    font-size: min(2.25rem, calc((100cqw - 1.5rem) / var(--aspect, 1)));
    /* Only the glyphs follow the colour picker; labels stay readable. */
    color: var(--icon-colour, inherit);
  }}
  /* A colour input is a square with a chrome border by default; strip it back
     so it sits in the swatch row as one more circle. */
  #picker {{
    -webkit-appearance: none;
    appearance: none;
    padding: 0;
    border: none;
    background: none;
    border-radius: 9999px;
    overflow: hidden;
  }}
  #picker::-webkit-color-swatch-wrapper {{ padding: 0; }}
  #picker::-webkit-color-swatch {{ border: none; border-radius: 9999px; }}
  #picker::-moz-color-swatch {{ border: none; border-radius: 9999px; }}
</style>
<script>
  // Set before first paint so the page does not flash the wrong background.
  (() => {{
    // Light by default: icons are drawn for a white page, so that is the
    // honest first impression. The dark switch is there to check them against
    // the other background.
    let saved = null;
    try {{ saved = localStorage.getItem('icofon-theme'); }} catch {{}}
    document.documentElement.dataset.theme = saved || 'light';
  }})();
</script>
</head>
<body class="bg-white text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
<div class="mx-auto max-w-6xl px-4 pb-8 sm:px-8">

<header class="sticky top-0 z-10 mb-7 flex flex-wrap items-end justify-between gap-4
               border-b border-zinc-200 bg-white/85 pt-8 pb-5 backdrop-blur
               dark:border-zinc-800 dark:bg-zinc-950/85">
  <div>
    <h1 class="text-xl font-semibold tracking-tight">{family}</h1>
    <p class="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
      <span id="shown">{total}</span> of {total} icons
    </p>
  </div>
  <div class="flex w-full max-w-sm items-center gap-2">
    <input id="search" type="search" placeholder="Search icons…" autocomplete="off" autofocus
           class="min-w-0 flex-1 rounded-lg border border-zinc-200 bg-white px-3 py-2 text-sm
                  placeholder:text-zinc-400 focus:border-transparent focus:outline-2
                  focus:outline-blue-600 dark:border-zinc-800 dark:bg-zinc-900
                  dark:placeholder:text-zinc-500 dark:focus:outline-blue-400">
    <div class="flex shrink-0 rounded-lg border border-zinc-200 p-0.5 dark:border-zinc-800">
      <button type="button" data-theme-set="light" title="Light background"
              class="theme-btn cursor-pointer rounded-md px-2 py-1 text-sm">☀</button>
      <button type="button" data-theme-set="dark" title="Dark background"
              class="theme-btn cursor-pointer rounded-md px-2 py-1 text-sm">☾</button>
    </div>
  </div>
  <div class="flex w-full items-center gap-2">
    <span class="text-xs text-zinc-500 dark:text-zinc-400">Colour</span>
    <div id="swatches" class="flex flex-wrap items-center gap-1.5">
      <button type="button" data-colour="" title="Follow the page"
              class="swatch size-6 cursor-pointer rounded-full border border-zinc-300
                     bg-zinc-900 dark:border-zinc-700 dark:bg-zinc-100"></button>
{swatches}    </div>
    <input id="picker" type="color" value="#2563eb" title="Pick any colour"
           class="size-6 shrink-0 cursor-pointer rounded-full ring-1 ring-black/10
                  dark:ring-white/15">
    <span id="colour-note" class="text-xs text-zinc-400 dark:text-zinc-500"></span>
  </div>
</header>

<main>
"##,
        family = family,
        css = escape(css_url),
        tailwind = TAILWIND_CDN,
        total = icons.len(),
        swatches = SWATCHES
            .iter()
            .map(|colour| format!(
                "      <button type=\"button\" data-colour=\"{colour}\" title=\"{colour}\"\n\
                 \x20             class=\"swatch size-6 cursor-pointer rounded-full border \
                 border-black/10 dark:border-white/15\" style=\"background:{colour}\"></button>\n"
            ))
            .collect::<String>(),
    ));

    // Icons arrive sorted by group, so consecutive runs share a section.
    for (group, members) in group_runs(icons) {
        page.push_str(&section_open(group));
        for icon in members {
            page.push_str(&card(icon, prefix));
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

fn card(icon: &Icon, prefix: &str) -> String {
    let class = format!("{prefix}-{}", icon.name);
    let code = icon.codepoint as u32;
    // How many ems wide the glyph is. A wide icon would otherwise run straight
    // out of its card, so the stylesheet divides the display size by this.
    let aspect = f64::from(icon.outline.advance) / f64::from(crate::font::UNITS_PER_EM);
    // A colour icon paints its own colours, so it will not follow the CSS
    // `color` the way the rest do. Worth saying on the card.
    let colour_note = if icon.outline.layers.is_empty() {
        String::new()
    } else {
        r#"
        <code class="font-mono text-[10px] text-zinc-400 dark:text-zinc-500">fixed colour</code>"#
            .to_string()
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
        r#"    <figure data-search="{search}" style="--aspect:{aspect:.3}"
            class="card m-0 flex flex-col items-center gap-3 rounded-xl border border-zinc-200
                   px-3 pt-5 pb-3.5 text-center dark:border-zinc-800">
      <span class="glyph {class} leading-none" aria-hidden="true"></span>
      <figcaption class="flex w-full flex-col items-center gap-1">
        <span class="max-w-full text-[13px] font-medium break-all">{name}</span>{width_note}{colour_note}
        <button type="button" data-copy="{class}" title="Copy class name"
                class="name max-w-full cursor-pointer rounded-md px-1.5 py-0.5 font-mono text-[12px]
                       break-all text-zinc-500 hover:bg-blue-600/10 hover:text-blue-700
                       dark:text-zinc-400 dark:hover:bg-blue-400/15
                       dark:hover:text-blue-300">.{class}</button>
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
                if icon.outline.layers.is_empty() {
                    ""
                } else {
                    "colour color"
                },
            ]
            .iter()
            .filter(|part| !part.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
        ),
        class = escape(&class),
        name = escape(&icon.label),
        code = code,
        aspect = aspect,
        width_note = width_note,
        colour_note = colour_note,
    )
}

/// Preset colours for the preview. Enough to check an icon against a light and
/// a dark foreground and a few brand-ish hues, without turning into a palette
/// editor.
const SWATCHES: [&str; 7] = [
    "#71717a", "#2563eb", "#0d9488", "#16a34a", "#ca8a04", "#dc2626", "#9333ea",
];

const SCRIPT: &str = r#"  const cards = Array.from(document.querySelectorAll('.card'));
  const groups = Array.from(document.querySelectorAll('.group'));
  const search = document.getElementById('search');
  const shown = document.getElementById('shown');
  const empty = document.getElementById('empty');
  const toast = document.getElementById('toast');

  // The picker sets a custom property on <main> that only .glyph reads, so the
  // icons follow it while the card text stays readable. Icons drawn with
  // currentColor pick it up; COLR icons keep the colours baked into the font.
  const grid = document.querySelector('main');
  const swatches = Array.from(document.querySelectorAll('.swatch'));
  const picker = document.getElementById('picker');
  const colourNote = document.getElementById('colour-note');

  function applyColour(colour) {
    grid.style.setProperty('--icon-colour', colour || 'inherit');
    colourNote.textContent = colour || '';
    for (const swatch of swatches) {
      const on = (swatch.dataset.colour || '') === (colour || '');
      swatch.classList.toggle('ring-2', on);
      swatch.classList.toggle('ring-blue-500', on);
      swatch.classList.toggle('ring-offset-1', on);
    }
    try { localStorage.setItem('icofon-colour', colour || ''); } catch {}
  }
  for (const swatch of swatches) {
    swatch.addEventListener('click', () => applyColour(swatch.dataset.colour || ''));
  }
  picker.addEventListener('input', () => applyColour(picker.value));
  let storedColour = '';
  try { storedColour = localStorage.getItem('icofon-colour') || ''; } catch {}
  applyColour(storedColour);

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
      paintTheme();
    });
  }
  paintTheme();

  search.addEventListener('input', () => {
    const query = search.value.trim().toLowerCase();
    let visible = 0;
    for (const card of cards) {
      const match = !query || card.dataset.search.includes(query);
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
  });

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
    fn lists_every_icon_with_its_class_and_codepoint() {
        let icons = vec![icon("arrow-left", '\u{e900}'), icon("star", '\u{e9f0}')];
        let page = render(&icons, "My Icons", "ico", "icons.css");

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
    fn the_header_offers_a_colour_palette() {
        let page = render(&[icon("plain", '\u{e900}')], "Icons", "icon", "f.css");
        for swatch in SWATCHES {
            assert!(
                page.contains(&format!(r#"data-colour="{swatch}""#)),
                "{swatch}"
            );
        }
        // Plus "follow the page" and a free-form picker.
        assert!(page.contains(r#"data-colour="""#));
        assert!(page.contains(r#"id="picker" type="color""#));
    }

    #[test]
    fn colour_icons_are_marked_and_searchable() {
        let mut colourful = icon("brand", '\u{e900}');
        colourful.outline.layers.push(crate::svg::Layer {
            path: kurbo::BezPath::new(),
            paint: crate::svg::LayerPaint::Foreground,
        });
        let page = render(&[colourful], "Icons", "icon", "f.css");
        assert!(page.contains("fixed colour"));
        assert!(page.contains("colour color"));
    }

    #[test]
    fn plain_icons_are_not_marked_as_colour() {
        let page = render(&[icon("plain", '\u{e900}')], "Icons", "icon", "f.css");
        assert!(!page.contains("fixed colour"));
    }

    #[test]
    fn wide_icons_carry_their_aspect_so_the_card_can_shrink_them() {
        let mut wide = icon("wordmark", '\u{e900}');
        wide.outline.advance = 16_538;
        let page = render(&[wide], "Icons", "icon", "f.css");
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
        let page = render(&[icon("square", '\u{e900}')], "Icons", "icon", "f.css");
        assert!(page.contains("--aspect:1.000"));
        assert!(!page.contains("× wide"));
    }

    #[test]
    fn pulls_tailwind_from_the_cdn_so_the_page_needs_no_build() {
        let page = render(&[icon("ok", '\u{e900}')], "Icons", "icon", "f.css");
        assert!(page.contains(&format!(r#"<script src="{TAILWIND_CDN}"></script>"#)));
    }

    #[test]
    fn search_index_covers_name_class_and_code() {
        let page = render(&[icon("arrow-left", '\u{e900}')], "Icons", "icon", "f.css");
        assert!(page.contains(r#"data-search="arrow-left icon-arrow-left e900""#));
    }

    #[test]
    fn a_grouped_card_shows_the_leaf_name_and_the_full_class() {
        // The heading already says "arrows", so repeating it in the title would
        // just be noise; the class still carries the complete name.
        let page = render(
            &[grouped("left", '\u{e901}', Some("arrows"))],
            "Icons",
            "icon",
            "f.css",
        );
        assert!(page.contains(">left</span>"), "card title is the leaf");
        assert!(page.contains(">.icon-arrows-left</button>"));
        assert!(page.contains(r#"data-copy="icon-arrows-left""#));
    }

    #[test]
    fn subfolders_become_labelled_sections() {
        let icons = vec![
            icon("loose", '\u{e900}'),
            grouped("left", '\u{e901}', Some("arrows")),
            grouped("right", '\u{e902}', Some("arrows")),
            grouped("share", '\u{e903}', Some("social")),
        ];
        let page = render(&icons, "Icons", "icon", "f.css");

        // One section per group, plus the unlabelled one for top-level icons.
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
    fn a_flat_folder_still_renders_one_unlabelled_section() {
        let page = render(&[icon("only", '\u{e900}')], "Icons", "icon", "f.css");
        assert_eq!(page.matches("<section").count(), 1);
        assert!(!page.contains("<h2"));
    }

    #[test]
    fn the_group_is_searchable() {
        let page = render(
            &[grouped("left", '\u{e900}', Some("arrows"))],
            "Icons",
            "icon",
            "f.css",
        );
        assert!(page.contains(r#"data-search="arrows-left icon-arrows-left e900 arrows""#));
    }

    #[test]
    fn markup_from_user_supplied_names_is_escaped() {
        let page = render(
            &[icon("ok", '\u{e900}')],
            "<script>alert(1)</script>",
            "icon",
            "f.css",
        );
        assert!(!page.contains("<script>alert(1)</script>"));
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }
}
