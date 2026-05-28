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
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{family} — icons</title>
<link rel="stylesheet" href="{css}">
<script src="{tailwind}"></script>
</head>
<body class="bg-white text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
<div class="mx-auto max-w-6xl px-4 py-8 sm:px-8">

<header class="mb-7 flex flex-wrap items-end justify-between gap-4 border-b border-zinc-200 pb-5 dark:border-zinc-800">
  <div>
    <h1 class="text-xl font-semibold tracking-tight">{family}</h1>
    <p class="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
      <span id="shown">{total}</span> of {total} icons
    </p>
  </div>
  <input id="search" type="search" placeholder="Search icons…" autocomplete="off" autofocus
         class="w-full max-w-sm rounded-lg border border-zinc-200 bg-white px-3 py-2 text-sm
                placeholder:text-zinc-400 focus:border-transparent focus:outline-2
                focus:outline-blue-600 dark:border-zinc-800 dark:bg-zinc-900
                dark:placeholder:text-zinc-500 dark:focus:outline-blue-400">
</header>

<main>
"#,
        family = family,
        css = escape(css_url),
        tailwind = TAILWIND_CDN,
        total = icons.len(),
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
    format!(
        r#"    <figure data-search="{search}"
            class="card m-0 flex flex-col items-center gap-3 rounded-xl border border-zinc-200
                   px-3 pt-5 pb-3.5 text-center dark:border-zinc-800">
      <span class="{class} text-4xl leading-none" aria-hidden="true"></span>
      <figcaption class="flex w-full flex-col items-center gap-1">
        <span class="max-w-full text-[13px] font-medium break-all">{name}</span>
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
            ]
            .iter()
            .filter(|part| !part.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
        ),
        class = escape(&class),
        name = escape(&icon.name),
        code = code,
    )
}

const SCRIPT: &str = r#"  const cards = Array.from(document.querySelectorAll('.card'));
  const groups = Array.from(document.querySelectorAll('.group'));
  const search = document.getElementById('search');
  const shown = document.getElementById('shown');
  const empty = document.getElementById('empty');
  const toast = document.getElementById('toast');

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

    fn grouped(name: &str, codepoint: char, group: Option<&str>) -> Icon {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                        <rect width="24" height="24" fill="#000"/>
                      </svg>"##;
        Icon {
            name: name.to_string(),
            group: group.map(str::to_string),
            codepoint,
            outline: svg::parse(svg.as_bytes(), name).unwrap(),
        }
    }

    #[test]
    fn lists_every_icon_with_its_class_and_codepoint() {
        let icons = vec![icon("arrow-left", '\u{e900}'), icon("star", '\u{e9f0}')];
        let page = render(&icons, "My Icons", "ico", "icons.css");

        assert!(page.contains(r#"<link rel="stylesheet" href="icons.css">"#));
        // Each card carries the glyph, the bare name, the class and the codepoint.
        assert!(page.contains(r#"<span class="ico-arrow-left "#));
        assert!(page.contains(">arrow-left</span>"));
        assert!(page.contains(r#"data-copy="ico-arrow-left""#));
        assert!(page.contains(">.ico-arrow-left</button>"));
        assert!(page.contains("U+E900"));
        assert!(page.contains(r#"<span class="ico-star "#));
        assert!(page.contains("U+E9F0"));
        assert_eq!(page.matches("data-search=").count(), icons.len());
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
        assert!(page.contains(r#"data-search="left icon-left e900 arrows""#));
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
