//! icofon — build an icon font (TTF + CSS) from a folder of SVG files.

mod css;
mod font;
mod html;
mod manifest;
mod svg;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

use font::Icon;
use manifest::Manifest;

#[derive(Parser)]
#[command(
    name = "icofon",
    version,
    about = "Build an icon font and matching stylesheet from a folder of SVG files"
)]
struct Args {
    /// Folder containing the SVG icons.
    input: PathBuf,

    /// Path of the TrueType font to write.
    output: PathBuf,

    /// Path of the stylesheet to write (defaults to the font path with a .css extension).
    #[arg(long, value_name = "PATH")]
    css: Option<PathBuf>,

    /// Path of the preview page to write (defaults to example.html beside the stylesheet).
    #[arg(long, value_name = "PATH")]
    html: Option<PathBuf>,

    /// Skip writing the preview page.
    #[arg(long, conflicts_with = "html")]
    no_html: bool,

    /// Path of the codepoint manifest, which keeps codepoints stable across
    /// builds (defaults to icofon.json inside the icon folder).
    #[arg(long, value_name = "PATH")]
    manifest: Option<PathBuf>,

    /// Do not read or write the codepoint manifest. Codepoints are then assigned
    /// from scratch on every build and will move as icons are added.
    #[arg(long, conflicts_with = "manifest")]
    no_manifest: bool,

    /// Font family name used in the font and in the CSS (defaults to the font file name).
    #[arg(long, value_name = "NAME")]
    font_family: Option<String>,

    /// Prefix for the generated CSS classes.
    #[arg(long, default_value = "icon", value_name = "PREFIX")]
    prefix: String,

    /// First codepoint to assign, as hex. Defaults to the start of the Private
    /// Use Area block that icon fonts conventionally use.
    #[arg(long, default_value = "e900", value_name = "HEX", value_parser = parse_codepoint)]
    start: char,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let files = collect_svgs(&args.input)?;
    if files.is_empty() {
        bail!("no .svg files found in {}", args.input.display());
    }

    let family = args
        .font_family
        .clone()
        .or_else(|| file_stem(&args.output))
        .unwrap_or_else(|| "icofon".to_string());
    let css_path = args
        .css
        .clone()
        .unwrap_or_else(|| args.output.with_extension("css"));
    let html_path = (!args.no_html).then(|| {
        args.html.clone().unwrap_or_else(|| {
            css_path
                .parent()
                .unwrap_or(Path::new("."))
                .join("example.html")
        })
    });

    let manifest_path = (!args.no_manifest).then(|| {
        args.manifest
            .clone()
            .unwrap_or_else(|| args.input.join(manifest::DEFAULT_FILE))
    });
    let mut manifest = match &manifest_path {
        Some(path) => Manifest::load(path)?,
        None => Manifest::default(),
    };

    let icons = load_icons(&files, args.start, &manifest)?;

    let font = font::build(&icons, &family)?;
    write(&args.output, &font)?;
    // Both paths must exist before the url between them can be resolved, so the
    // stylesheet's directory is created up front rather than at write time.
    ensure_parent(&css_path)?;
    let font_url = relative_url(&css_path, &args.output);
    write(
        &css_path,
        css::render(&icons, &family, &args.prefix, &font_url).as_bytes(),
    )?;

    let mut written = format!("{} + {}", args.output.display(), css_path.display());
    if let Some(html_path) = &html_path {
        ensure_parent(html_path)?;
        let css_url = relative_url(html_path, &css_path);
        write(
            html_path,
            html::render(&icons, &family, &args.prefix, &css_url).as_bytes(),
        )?;
        written.push_str(&format!(" + {}", html_path.display()));
    }

    if let Some(manifest_path) = &manifest_path {
        for icon in &icons {
            manifest.insert(&icon.name, icon.codepoint);
        }
        ensure_parent(manifest_path)?;
        manifest.save(manifest_path)?;
        written.push_str(&format!(" + {}", manifest_path.display()));
    }

    println!("{} icons -> {written}", icons.len());
    Ok(())
}

/// An SVG found in the icon folder, with the subfolder it came from.
struct SvgFile {
    path: PathBuf,
    /// Path of the containing subfolder relative to the icon folder, used to
    /// group the preview page. `None` for icons sitting at the top level.
    group: Option<String>,
}

/// Every `.svg` in `dir` and its subfolders.
///
/// Sorted with top-level icons first, then by subfolder, then by file name, so
/// that both the preview page and any automatic codepoint assignment come out
/// the same on every run.
fn collect_svgs(dir: &Path) -> Result<Vec<SvgFile>> {
    let mut files = Vec::new();
    walk(dir, None, &mut files)?;
    files.sort_by(|a, b| {
        // `None` sorts before `Some`, which puts ungrouped icons first.
        (&a.group, &a.path).cmp(&(&b.group, &b.path))
    });
    Ok(files)
}

fn walk(dir: &Path, group: Option<&str>, out: &mut Vec<SvgFile>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;

    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        // Skip dotfiles and dot-directories, which are housekeeping, not icons.
        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            let nested = match group {
                Some(parent) => format!("{parent}/{name}"),
                None => name,
            };
            walk(&path, Some(&nested), out)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        {
            out.push(SvgFile {
                path,
                group: group.map(str::to_string),
            });
        }
    }
    Ok(())
}

/// Parse each SVG and pair it with a name and a codepoint.
///
/// Codepoints come from three places, in order of precedence: a `uE901-` prefix
/// on the file name, the manifest's record of a previous build, and finally the
/// next free codepoint at or after `first`. The manifest is what keeps an icon's
/// codepoint from moving when new icons are added around it.
fn load_icons(files: &[SvgFile], first: char, manifest: &Manifest) -> Result<Vec<Icon>> {
    let mut names: BTreeMap<String, &Path> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    let mut pinned_by: BTreeMap<char, String> = BTreeMap::new();

    // Resolve names and pinned codepoints first, so that a pinned codepoint is
    // never stolen by an icon that happens to be processed earlier.
    let mut pending = Vec::with_capacity(files.len());
    for file in files {
        let stem = file_stem(&file.path)
            .with_context(|| format!("{} has no file name", file.path.display()))?;
        let (codepoint, name) = split_codepoint(&stem);
        let name = sanitize_name(&name);
        if name.is_empty() {
            bail!("{} has no usable icon name", file.path.display());
        }
        if let Some(first_seen) = names.insert(name.clone(), &file.path) {
            bail!(
                "two icons would both be called '{name}':\n  {}\n  {}\n\
                 Icon names ignore subfolders, so rename one of them.",
                first_seen.display(),
                file.path.display()
            );
        }
        if let Some(codepoint) = codepoint {
            if !taken.insert(codepoint) {
                bail!("codepoint U+{:04X} is claimed twice", codepoint as u32);
            }
            pinned_by.insert(codepoint, name.clone());
        }
        pending.push((file, name, codepoint));
    }

    // Reserve everything the manifest has ever handed out, including to icons
    // that have since been deleted, so a codepoint is never reused.
    for codepoint in manifest.reserved() {
        if let Some(owner) = pinned_by.get(&codepoint) {
            // The pin wins, but only silently when it agrees with the record.
            if manifest.get(owner) != Some(codepoint) {
                bail!(
                    "U+{:04X} is pinned by a file name but the manifest already gave it to \
                     another icon; remove the pin or the manifest entry",
                    codepoint as u32
                );
            }
        }
        taken.insert(codepoint);
    }

    let mut next = first;
    let mut icons = Vec::with_capacity(pending.len());
    for (file, name, pinned) in pending {
        let codepoint = match pinned.or_else(|| manifest.get(&name)) {
            Some(codepoint) => codepoint,
            None => {
                let free = next_free(next, &taken)
                    .with_context(|| format!("ran out of codepoints at '{name}'"))?;
                taken.insert(free);
                next = char::from_u32(free as u32 + 1).unwrap_or(free);
                free
            }
        };
        icons.push(Icon {
            name,
            group: file.group.clone(),
            codepoint,
            outline: svg::load(&file.path)?,
        });
    }
    Ok(icons)
}

/// The first codepoint at or after `from` that no icon has claimed.
fn next_free(from: char, taken: &BTreeSet<char>) -> Option<char> {
    (from as u32..=char::MAX as u32)
        .filter_map(char::from_u32)
        .find(|c| !taken.contains(c))
}

/// Split an optional `uE901-` / `U+E901_` prefix off a file stem.
///
/// The `u` is required: without it a perfectly ordinary icon named `1f-arrow`
/// would silently become a codepoint.
fn split_codepoint(stem: &str) -> (Option<char>, String) {
    let rest = stem
        .strip_prefix("U+")
        .or_else(|| stem.strip_prefix("u+"))
        .or_else(|| stem.strip_prefix('u'))
        .or_else(|| stem.strip_prefix('U'));
    let Some(rest) = rest else {
        return (None, stem.to_string());
    };

    let split = rest.find(['-', '_', ' ']).unwrap_or(rest.len());
    let (hex, name) = rest.split_at(split);
    if !(4..=6).contains(&hex.len()) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return (None, stem.to_string());
    }

    match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
        Some(codepoint) => (
            Some(codepoint),
            name.trim_start_matches(['-', '_', ' ']).to_string(),
        ),
        None => (None, stem.to_string()),
    }
}

/// Reduce a file stem to something safe to paste into a CSS class name.
fn sanitize_name(stem: &str) -> String {
    let mut name = String::with_capacity(stem.len());
    for ch in stem.chars() {
        match ch {
            'a'..='z' | '0'..='9' | '-' | '_' => name.push(ch),
            'A'..='Z' => name.push(ch.to_ascii_lowercase()),
            _ => name.push('-'),
        }
    }
    // Collapse runs of separators and trim them off the ends.
    let mut collapsed = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(ch);
    }
    collapsed.trim_matches('-').to_string()
}

fn parse_codepoint(value: &str) -> Result<char, String> {
    let hex = value
        .trim_start_matches("U+")
        .trim_start_matches("u+")
        .trim_start_matches("0x");
    u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| format!("'{value}' is not a Unicode codepoint in hex"))
}

/// A url pointing at `target` from the page or stylesheet at `from`, so that
/// generated files keep referring to each other wherever they are deployed.
///
/// Falls back to the bare file name, which is right for the common case of both
/// files sharing a directory.
fn relative_url(from: &Path, target: &Path) -> String {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let from_dir = from.parent().unwrap_or(Path::new("."));
    let relative = std::fs::canonicalize(from_dir)
        .ok()
        .zip(std::fs::canonicalize(target).ok())
        .and_then(|(dir, target)| relative_path(&dir, &target));

    escape_url(&relative.unwrap_or(name))
}

/// Express `target` relative to the directory `base`. Both must be absolute.
fn relative_path(base: &Path, target: &Path) -> Option<String> {
    let mut base = base.components().peekable();
    let mut target = target.components().peekable();
    while base.peek().is_some() && base.peek() == target.peek() {
        base.next();
        target.next();
    }

    let rest: Vec<String> = target
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if rest.is_empty() {
        return None;
    }

    let mut parts = vec!["..".to_string(); base.count()];
    parts.extend(rest);
    Some(parts.join("/"))
}

fn escape_url(url: &str) -> String {
    url.replace('%', "%25")
        .replace(' ', "%20")
        .replace('\'', "%27")
        .replace('"', "%22")
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().into_owned())
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                              <rect width="24" height="24" fill="#000"/>
                            </svg>"##;

    /// A throwaway icon folder. Named per test so they can run in parallel.
    fn icon_folder(test: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("icofon-test-{test}"));
        std::fs::remove_dir_all(&dir).ok();
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, SQUARE).unwrap();
        }
        dir
    }

    fn codepoints(icons: &[Icon]) -> BTreeMap<&str, char> {
        icons
            .iter()
            .map(|i| (i.name.as_str(), i.codepoint))
            .collect()
    }

    #[test]
    fn subfolders_become_groups_without_touching_names() {
        let dir = icon_folder(
            "groups",
            &["check.svg", "arrows/left.svg", "arrows/nested/up.svg"],
        );
        let files = collect_svgs(&dir).unwrap();
        let icons = load_icons(&files, '\u{e900}', &Manifest::default()).unwrap();

        let groups: Vec<_> = icons
            .iter()
            .map(|i| (i.name.as_str(), i.group.as_deref()))
            .collect();
        assert_eq!(
            groups,
            [
                // Top-level icons first, then subfolders in path order.
                ("check", None),
                ("left", Some("arrows")),
                ("up", Some("arrows/nested")),
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_folders_cannot_hold_the_same_icon_name() {
        let dir = icon_folder("collide", &["arrows/left.svg", "social/left.svg"]);
        let files = collect_svgs(&dir).unwrap();
        let error = load_icons(&files, '\u{e900}', &Manifest::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("would both be called 'left'"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn adding_an_icon_leaves_existing_codepoints_alone() {
        let dir = icon_folder("stable", &["check.svg", "zoom.svg"]);
        let files = collect_svgs(&dir).unwrap();
        let first = load_icons(&files, '\u{e900}', &Manifest::default()).unwrap();
        let before = codepoints(&first);
        assert_eq!(before["check"], '\u{e900}');
        assert_eq!(before["zoom"], '\u{e901}');

        let mut manifest = Manifest::default();
        for icon in &first {
            manifest.insert(&icon.name, icon.codepoint);
        }

        // This one sorts before both existing icons, so without the manifest it
        // would take U+E900 and shift everything after it.
        std::fs::write(dir.join("aaa.svg"), SQUARE).unwrap();
        let files = collect_svgs(&dir).unwrap();
        let second = load_icons(&files, '\u{e900}', &manifest).unwrap();
        let after = codepoints(&second);

        assert_eq!(after["check"], '\u{e900}');
        assert_eq!(after["zoom"], '\u{e901}');
        assert_eq!(after["aaa"], '\u{e902}');
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_deleted_icons_codepoint_is_never_handed_to_another_icon() {
        let mut manifest = Manifest::default();
        manifest.insert("retired", '\u{e900}');

        let dir = icon_folder("retired", &["fresh.svg"]);
        let files = collect_svgs(&dir).unwrap();
        let icons = load_icons(&files, '\u{e900}', &manifest).unwrap();

        assert_eq!(icons[0].name, "fresh");
        assert_eq!(icons[0].codepoint, '\u{e901}', "U+E900 is still reserved");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pin_moves_the_icon_it_names() {
        // Renaming heart.svg to uE9F0-heart.svg is a deliberate instruction to
        // move that icon, so the pin wins over the icon's own record.
        let mut manifest = Manifest::default();
        manifest.insert("heart", '\u{e900}');

        let dir = icon_folder("pin-moves", &["uE9F0-heart.svg"]);
        let files = collect_svgs(&dir).unwrap();
        let icons = load_icons(&files, '\u{e900}', &manifest).unwrap();

        assert_eq!(icons[0].name, "heart");
        assert_eq!(icons[0].codepoint, '\u{e9f0}');
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pin_cannot_steal_another_icons_codepoint() {
        // U+E900 already belongs to heart, so letting badge pin it would leave
        // two icons fighting over one codepoint.
        let mut manifest = Manifest::default();
        manifest.insert("heart", '\u{e900}');

        let dir = icon_folder("pin-steals", &["uE900-badge.svg", "heart.svg"]);
        let files = collect_svgs(&dir).unwrap();
        let error = load_icons(&files, '\u{e900}', &manifest)
            .unwrap_err()
            .to_string();
        assert!(error.contains("pinned by a file name"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn explicit_codepoints_are_recognised() {
        assert_eq!(
            split_codepoint("uE901-heart"),
            (Some('\u{e901}'), "heart".to_string())
        );
        assert_eq!(
            split_codepoint("U+E901_heart"),
            (Some('\u{e901}'), "heart".to_string())
        );
    }

    #[test]
    fn names_that_only_look_like_codepoints_are_left_alone() {
        // No `u` prefix, so this is a name, not a codepoint.
        assert_eq!(
            split_codepoint("e901-heart"),
            (None, "e901-heart".to_string())
        );
        // Too short to be a codepoint.
        assert_eq!(split_codepoint("up-arrow"), (None, "up-arrow".to_string()));
        // Right prefix, but not hex.
        assert_eq!(
            split_codepoint("undo-zzzz"),
            (None, "undo-zzzz".to_string())
        );
    }

    #[test]
    fn names_are_reduced_to_css_safe_slugs() {
        assert_eq!(sanitize_name("Arrow Left"), "arrow-left");
        assert_eq!(sanitize_name("chevron--right"), "chevron-right");
        assert_eq!(sanitize_name("--trash--"), "trash");
        assert_eq!(sanitize_name("zoom_in (2)"), "zoom_in-2");
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        assert_eq!(
            relative_path(Path::new("/a/b/css"), Path::new("/a/b/font.ttf")).as_deref(),
            Some("../font.ttf")
        );
        assert_eq!(
            relative_path(Path::new("/a/b"), Path::new("/a/b/fonts/font.ttf")).as_deref(),
            Some("fonts/font.ttf")
        );
        assert_eq!(
            relative_path(Path::new("/a/b"), Path::new("/a/b/font.ttf")).as_deref(),
            Some("font.ttf")
        );
    }

    #[test]
    fn codepoint_flag_accepts_the_usual_spellings() {
        assert_eq!(parse_codepoint("e900"), Ok('\u{e900}'));
        assert_eq!(parse_codepoint("U+E900"), Ok('\u{e900}'));
        assert_eq!(parse_codepoint("0xe900"), Ok('\u{e900}'));
        assert!(parse_codepoint("nope").is_err());
    }
}
