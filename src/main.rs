//! icofon — build an icon font (TTF + CSS) from a folder of SVG files.

mod css;
mod font;
mod svg;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

use font::Icon;

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

    let icons = load_icons(&files, args.start)?;

    let font = font::build(&icons, &family)?;
    write(&args.output, &font)?;
    // Both paths must exist before the url between them can be resolved, so the
    // stylesheet's directory is created up front rather than at write time.
    ensure_parent(&css_path)?;
    let font_url = font_url(&css_path, &args.output);
    write(
        &css_path,
        css::render(&icons, &family, &args.prefix, &font_url).as_bytes(),
    )?;

    println!(
        "{} icons -> {} + {}",
        icons.len(),
        args.output.display(),
        css_path.display()
    );
    Ok(())
}

/// Every `.svg` directly inside `dir`, sorted by file name so that automatic
/// codepoint assignment is stable across runs.
fn collect_svgs(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Parse each SVG and pair it with a name and a codepoint.
///
/// A file named `uE901-heart.svg` pins its own codepoint; everything else is
/// handed the next free one starting from `first`.
fn load_icons(files: &[PathBuf], first: char) -> Result<Vec<Icon>> {
    let mut names = BTreeSet::new();
    let mut taken = BTreeSet::new();

    // Resolve names and explicit codepoints first, so that an explicitly
    // requested codepoint is never stolen by an earlier auto-assigned icon.
    let mut pending = Vec::with_capacity(files.len());
    for file in files {
        let stem =
            file_stem(file).with_context(|| format!("{} has no file name", file.display()))?;
        let (codepoint, name) = split_codepoint(&stem);
        let name = sanitize_name(&name);
        if name.is_empty() {
            bail!("{} has no usable icon name", file.display());
        }
        if !names.insert(name.clone()) {
            bail!("two icons would both be called '{name}'");
        }
        if let Some(codepoint) = codepoint
            && !taken.insert(codepoint)
        {
            bail!("codepoint U+{:04X} is claimed twice", codepoint as u32);
        }
        pending.push((file, name, codepoint));
    }

    let mut next = first;
    let mut icons = Vec::with_capacity(pending.len());
    for (file, name, codepoint) in pending {
        let codepoint = match codepoint {
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
            codepoint,
            outline: svg::load(file)?,
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

/// The `url()` to put in the stylesheet: the font's location relative to the
/// stylesheet, so the pair keeps working wherever it is deployed.
///
/// Falls back to the bare file name, which is right for the common case of both
/// files sharing a directory.
fn font_url(css_path: &Path, font_path: &Path) -> String {
    let name = font_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "font.ttf".to_string());

    let css_dir = css_path.parent().unwrap_or(Path::new("."));
    let relative = std::fs::canonicalize(css_dir)
        .ok()
        .zip(std::fs::canonicalize(font_path).ok())
        .and_then(|(dir, font)| relative_path(&dir, &font));

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
