//! icofon — build an icon font (TTF + CSS) from a folder of SVG files.

mod config;
mod css;
mod font;
mod html;
mod manifest;
mod svg;
mod webfont;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use config::{Format, OnDuplicate, OnError};
use font::Icon;
use manifest::Manifest;

#[derive(Parser)]
#[command(
  name = "icofon",
  version,
  about = "Build an icon font, stylesheet and preview page from a folder of SVG files",
  subcommand_required = true,
  arg_required_else_help = true
)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Build the fonts, the stylesheet and the preview page.
  Build(BuildArgs),
  /// Write an icofon.toml holding the settings this build would use.
  Init(InitArgs),
  /// Convert every icon and report, without writing anything.
  Check(BuildArgs),
  /// Build, then build again whenever an icon changes.
  Watch(BuildArgs),
}

/// Flags shared by build, check and watch. Every one is optional: what is not
/// given here comes from icofon.toml, and what neither gives comes from the
/// defaults in `settings`.
#[derive(clap::Args, Clone)]
struct BuildArgs {
  /// Folder holding the SVG icons.
  source: Option<PathBuf>,

  /// Folder to write the fonts, stylesheet and preview page into.
  #[arg(short, long, value_name = "DIR")]
  out: Option<PathBuf>,

  /// Base name for the generated files, and the font family name.
  #[arg(long, value_name = "NAME")]
  name: Option<String>,

  /// Font containers to write, smallest first in the stylesheet.
  #[arg(long, value_delimiter = ',', value_name = "LIST")]
  formats: Option<Vec<Format>>,

  /// Prefix for the generated CSS classes.
  #[arg(long, value_name = "PREFIX")]
  prefix: Option<String>,

  /// Require the prefix as a class of its own, so an icon is written
  /// `class="icon icon-arrow-left"`. Without this the stylesheet matches any
  /// class starting with the prefix, which also claims classes of your own
  /// that happen to start the same way.
  #[arg(long)]
  base_class: bool,

  /// Skip the preview page.
  #[arg(long)]
  no_preview: bool,

  /// Path of the codepoint manifest, which keeps codepoints stable across
  /// builds. Defaults to icofon.json inside the icon folder.
  #[arg(long, value_name = "PATH")]
  manifest: Option<PathBuf>,

  /// Assign codepoints from scratch every build instead of keeping a manifest.
  /// They will move as icons are added, which breaks pages already using the
  /// font.
  #[arg(long, conflicts_with = "manifest")]
  no_manifest: bool,

  /// What to do about an icon that cannot become a glyph.
  #[arg(long, value_name = "WHAT")]
  on_error: Option<OnError>,

  /// What to do about two files whose names reduce to the same icon name.
  #[arg(long, value_name = "WHAT")]
  on_duplicate: Option<OnDuplicate>,

  /// First codepoint to assign, as hex. Defaults to the start of the Private
  /// Use Area block that icon fonts conventionally use.
  #[arg(long, value_name = "HEX")]
  start: Option<String>,

  /// Read this config file instead of looking for icofon.toml.
  #[arg(long, value_name = "PATH")]
  config: Option<PathBuf>,
}

#[derive(clap::Args)]
struct InitArgs {
  /// Folder holding the SVG icons.
  source: Option<PathBuf>,

  /// Folder to write the fonts, stylesheet and preview page into.
  #[arg(short, long, value_name = "DIR")]
  out: Option<PathBuf>,

  /// Overwrite an existing icofon.toml.
  #[arg(long)]
  force: bool,
}

fn main() -> Result<()> {
  match Cli::parse().command {
    Command::Build(args) => {
      let report = build(&settings(&args)?, Write::Files)?;
      println!("{report}");
      Ok(())
    }
    Command::Check(args) => {
      let report = build(&settings(&args)?, Write::Nothing)?;
      println!("{report}");
      Ok(())
    }
    Command::Watch(args) => watch(&settings(&args)?),
    Command::Init(args) => init(&args),
  }
}

/// Merge the defaults, the config file and the flags into one set of settings.
fn settings(args: &BuildArgs) -> Result<config::Build> {
  let cwd = std::env::current_dir()?;
  let (file, base) = match &args.config {
    Some(path) => {
      let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
      (config::File::load(path)?, dir)
    }
    None => match config::File::discover(&cwd)? {
      Some((file, dir)) => (file, dir),
      None => (config::File::default(), cwd.clone()),
    },
  };

  // Paths in the file are relative to the file, so a build behaves the same
  // from any directory in the project.
  let rooted = |path: PathBuf| {
    if path.is_absolute() {
      path
    } else {
      base.join(path)
    }
  };

  let source = args
    .source
    .clone()
    .or_else(|| file.source.clone().map(&rooted))
    .context("no icon folder given, and no source set in icofon.toml")?;
  let out = args
    .out
    .clone()
    .or_else(|| file.out.clone().map(&rooted))
    .unwrap_or_else(|| PathBuf::from("dist"));
  let name = args
    .name
    .clone()
    .or_else(|| file.name.clone())
    .or_else(|| file_stem(&source))
    .unwrap_or_else(|| "icofon".to_string());

  let manifest = if args.no_manifest || file.manifest == Some(false) {
    None
  } else {
    Some(
      args
        .manifest
        .clone()
        .or_else(|| file.manifest_path.clone().map(&rooted))
        .unwrap_or_else(|| source.join(manifest::DEFAULT_FILE)),
    )
  };

  let start = match args.start.clone().or_else(|| file.start.clone()) {
    Some(hex) => parse_codepoint(&hex).map_err(anyhow::Error::msg)?,
    None => '\u{e900}',
  };

  Ok(config::Build {
    source,
    out,
    name,
    formats: args
      .formats
      .clone()
      .or(file.formats)
      .unwrap_or_else(|| vec![Format::Woff2, Format::Woff, Format::Ttf]),
    prefix: args
      .prefix
      .clone()
      .or(file.prefix)
      .unwrap_or_else(|| "icon".to_string()),
    base_class: args.base_class || file.base_class.unwrap_or(false),
    preview: !args.no_preview && file.preview.unwrap_or(true),
    manifest,
    start,
    on_error: args.on_error.or(file.on_error).unwrap_or(OnError::Fail),
    on_duplicate: args
      .on_duplicate
      .or(file.on_duplicate)
      .unwrap_or(OnDuplicate::Fail),
  })
}

/// Whether a build writes its results or only reports them.
#[derive(Clone, Copy, PartialEq)]
enum Write {
  Files,
  Nothing,
}

/// What a build produced, in the shape it is printed.
struct Report {
  icons: usize,
  wrote: Vec<(PathBuf, usize)>,
  checked_only: bool,
}

impl std::fmt::Display for Report {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if self.checked_only {
      return write!(f, "{} icons convert cleanly", self.icons);
    }
    writeln!(f, "{} icons", self.icons)?;
    for (path, size) in &self.wrote {
      writeln!(f, "  {:<28} {}", short(path), human(*size))?;
    }
    Ok(())
  }
}

/// A path as it is worth reading back: relative to where the command was run,
/// since an absolute path from the config file tells the reader nothing.
fn short(path: &Path) -> String {
  let cleaned: PathBuf = path
    .components()
    .filter(|part| !matches!(part, std::path::Component::CurDir))
    .collect();
  std::env::current_dir()
    .ok()
    .and_then(|cwd| cleaned.strip_prefix(&cwd).ok().map(Path::to_path_buf))
    .unwrap_or(cleaned)
    .display()
    .to_string()
}

fn human(bytes: usize) -> String {
  if bytes < 1024 {
    format!("{bytes} B")
  } else {
    format!("{:.1} KB", bytes as f64 / 1024.0)
  }
}

/// Read the icons, build every requested format, and write what was asked for.
fn build(settings: &config::Build, write_mode: Write) -> Result<Report> {
  let files = collect_svgs(&settings.source)?;
  if files.is_empty() {
    bail!("no .svg files found in {}", settings.source.display());
  }

  let mut manifest = match &settings.manifest {
    Some(path) => Manifest::load(path)?,
    None => Manifest::default(),
  };

  let icons = load_icons(&files, settings.start, &manifest, settings.on_duplicate)?;
  let icons = triage(icons, settings.on_error == OnError::Skip)?;

  if write_mode == Write::Nothing {
    return Ok(Report {
      icons: icons.len(),
      wrote: Vec::new(),
      checked_only: true,
    });
  }

  let classes = css::Classes {
    prefix: &settings.prefix,
    base_class: settings.base_class,
  };
  let formats = settings.ordered_formats();

  let ttf = font::build(&icons, &settings.name)?;
  let css_path = settings.css_path();
  // Every path has to exist before the urls between them can be resolved.
  ensure_parent(&css_path)?;

  let mut wrote = Vec::new();
  let mut urls = Vec::new();
  for format in &formats {
    let bytes = match format {
      Format::Ttf => ttf.clone(),
      Format::Woff => webfont::woff(&ttf)?,
      Format::Woff2 => webfont::woff2(&ttf)?,
    };
    let path = settings.font_path(*format);
    write(&path, &bytes)?;
    urls.push((relative_url(&css_path, &path), format.css_format()));
    wrote.push((path, bytes.len()));
  }

  let sources: Vec<css::Source<'_>> = urls
    .iter()
    .map(|(url, format)| css::Source { url, format })
    .collect();
  let stylesheet = css::render(&icons, &settings.name, classes, &sources);
  write(&css_path, stylesheet.as_bytes())?;
  wrote.push((css_path.clone(), stylesheet.len()));

  if settings.preview {
    let path = settings.preview_path();
    ensure_parent(&path)?;
    let css_url = relative_url(&path, &css_path);
    let page = html::render(&icons, &settings.name, classes, &css_url);
    write(&path, page.as_bytes())?;
    wrote.push((path, page.len()));
  }

  if let Some(path) = &settings.manifest {
    for icon in &icons {
      manifest.insert(
        &manifest_key(icon.group.as_deref(), &icon.source),
        &icon.name,
        icon.codepoint,
      );
    }
    ensure_parent(path)?;
    manifest.save(path)?;
    let size = std::fs::metadata(path)
      .map(|m| m.len() as usize)
      .unwrap_or(0);
    wrote.push((path.clone(), size));
  }

  Ok(Report {
    icons: icons.len(),
    wrote,
    checked_only: false,
  })
}

/// Build once, then rebuild on every change under the icon folder.
fn watch(settings: &config::Build) -> Result<()> {
  use notify::{RecursiveMode, Watcher};

  match build(settings, Write::Files) {
    Ok(report) => println!("{report}"),
    Err(error) => eprintln!("{error:?}"),
  }

  let (tx, rx) = std::sync::mpsc::channel();
  let mut watcher = notify::recommended_watcher(move |event| {
    let _ = tx.send(event);
  })?;
  watcher.watch(&settings.source, RecursiveMode::Recursive)?;
  println!("watching {} — ^C to stop", settings.source.display());

  loop {
    // Editors touch a file several times per save, so wait for the flurry to
    // stop before building rather than building once per event.
    let first = rx.recv();
    if first.is_err() {
      break;
    }
    while rx
      .recv_timeout(std::time::Duration::from_millis(150))
      .is_ok()
    {}

    match build(settings, Write::Files) {
      Ok(report) => println!("{report}"),
      Err(error) => eprintln!("{error:?}"),
    }
  }
  Ok(())
}

/// Write an icofon.toml describing what a build would do right now.
fn init(args: &InitArgs) -> Result<()> {
  let path = PathBuf::from(config::FILE);
  if path.exists() && !args.force {
    bail!(
      "{} already exists — pass --force to overwrite",
      path.display()
    );
  }

  let source: PathBuf = args
    .source
    .clone()
    .unwrap_or_else(|| PathBuf::from("icons"))
    .components()
    .filter(|part| !matches!(part, std::path::Component::CurDir))
    .collect();
  let file = config::File {
    source: Some(source.clone()),
    out: Some(args.out.clone().unwrap_or_else(|| PathBuf::from("dist"))),
    name: file_stem(&source),
    formats: Some(vec![Format::Woff2, Format::Woff, Format::Ttf]),
    prefix: Some("icon".to_string()),
    ..config::File::default()
  };

  let toml = toml::to_string_pretty(&file)?;
  std::fs::write(&path, &toml)?;
  println!("wrote {}", path.display());
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

/// A file and the name it asks for, before any clash between them is settled.
struct Wanted<'a> {
  file: &'a SvgFile,
  /// How the manifest knows this icon: its path inside the icon folder.
  key: String,
  /// The name the file would have if nothing else wanted it.
  base: String,
  /// The file part of that name, which is what the preview card shows.
  label: String,
  /// A codepoint asked for by a `uE901-` prefix on the file name.
  pin: Option<char>,
}

/// A file with its name settled.
struct Named<'a> {
  file: &'a SvgFile,
  key: String,
  name: String,
  label: String,
  pin: Option<char>,
}

/// Parse each SVG and pair it with a name and a codepoint.
///
/// An icon's name is its file stem, prefixed with the subfolders it sits in, so
/// `arrows/left.svg` is `arrows-left`. Two files whose stems reduce to the same
/// name stop the build unless `on_duplicate` asks for them to be numbered.
///
/// Codepoints come from three places, in order of precedence: a `uE901-` prefix
/// on the file name, the manifest's record of a previous build, and finally the
/// next free codepoint at or after `first`. The manifest is what keeps an icon's
/// codepoint from moving when new icons are added around it.
fn load_icons(
  files: &[SvgFile],
  first: char,
  manifest: &Manifest,
  on_duplicate: OnDuplicate,
) -> Result<Vec<Icon>> {
  let named = resolve_names(files, manifest, on_duplicate)?;

  // Resolve pinned codepoints before anything is assigned, so that a pin is
  // never stolen by an icon that happens to be processed earlier.
  let mut taken = BTreeSet::new();
  let mut pinned_by: BTreeMap<char, usize> = BTreeMap::new();
  for (index, icon) in named.iter().enumerate() {
    if let Some(pin) = icon.pin {
      if !taken.insert(pin) {
        bail!("codepoint U+{:04X} is claimed twice", pin as u32);
      }
      pinned_by.insert(pin, index);
    }
  }

  // Reserve everything the manifest has ever handed out, including to icons
  // that have since been deleted, so a codepoint is never reused.
  for codepoint in manifest.reserved() {
    if let Some(owner) = pinned_by.get(&codepoint).map(|index| &named[*index]) {
      // The pin wins, but only silently when it agrees with the record.
      if manifest.get(&owner.key, &owner.name) != Some(codepoint) {
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
  let mut icons = Vec::with_capacity(named.len());
  for icon in named {
    let codepoint = match icon.pin.or_else(|| manifest.get(&icon.key, &icon.name)) {
      Some(codepoint) => codepoint,
      None => {
        let free = next_free(next, &taken)
          .with_context(|| format!("ran out of codepoints at '{}'", icon.name))?;
        taken.insert(free);
        next = char::from_u32(free as u32 + 1).unwrap_or(free);
        free
      }
    };
    icons.push(Icon {
      name: icon.name,
      label: icon.label,
      group: icon.file.group.clone(),
      source: icon.file.path.clone(),
      codepoint,
      outline: svg::load(&icon.file.path)?,
    });
  }
  Ok(icons)
}

/// Give every file a name of its own.
///
/// Different file names can still reduce to the same slug — `map-pin.svg` and
/// `map_pin.svg` both give `map-pin` — which is a build failure by default,
/// because a number is not part of either file: which file gets which depends
/// on what else is in the folder. Under `--on-duplicate number` they are
/// numbered anyway, and the manifest remembers which file was given which
/// number, so a third clashing file takes the next one instead of renumbering
/// the icons that were there first.
fn resolve_names<'a>(
  files: &'a [SvgFile],
  manifest: &Manifest,
  on_duplicate: OnDuplicate,
) -> Result<Vec<Named<'a>>> {
  let mut wanted = Vec::with_capacity(files.len());
  for file in files {
    let stem =
      file_stem(&file.path).with_context(|| format!("{} has no file name", file.path.display()))?;
    let (pin, stem) = split_codepoint(&stem);
    let label = sanitize_name(&stem);
    // The subfolder becomes part of the name, so icons/arrows/left.svg is
    // `arrows-left`. Two folders can then each hold a `left.svg`.
    let base = match &file.group {
      Some(group) => sanitize_name(&format!("{group}/{stem}")),
      None => label.clone(),
    };
    if base.is_empty() {
      bail!("{} has no usable icon name", file.path.display());
    }
    wanted.push(Wanted {
      file,
      key: manifest_key(file.group.as_deref(), &file.path),
      base,
      label,
      pin,
    });
  }

  if on_duplicate == OnDuplicate::Fail {
    reject_clashes(&wanted)?;
  }

  // A name an earlier build gave this exact file is claimed first, whatever
  // position the file now sorts into. That is what makes numbering survive a
  // third clashing file: the icon that was `map-pin-2` stays `map-pin-2`, and
  // therefore keeps its class and its codepoint.
  let mut taken: BTreeMap<String, &Path> = BTreeMap::new();
  let mut settled: Vec<Option<String>> = vec![None; wanted.len()];
  for (slot, want) in settled.iter_mut().zip(&wanted) {
    let Some(recorded) = manifest.name(&want.key) else {
      continue;
    };
    if is_variant_of(recorded, &want.base) && !taken.contains_key(recorded) {
      taken.insert(recorded.to_string(), &want.file.path);
      *slot = Some(recorded.to_string());
    }
  }

  let mut named = Vec::with_capacity(wanted.len());
  for (want, settled) in wanted.iter().zip(settled) {
    let name = match settled {
      Some(name) => name,
      None => {
        let name = next_free_name(&want.base, &taken);
        if name != want.base {
          eprintln!(
            "'{}' is already taken by {}, so {} is called '{name}'",
            want.base,
            taken[&want.base].display(),
            want.file.path.display(),
          );
        }
        name
      }
    };
    taken.insert(name.clone(), &want.file.path);
    // A numbered name needs a numbered label too, or two preview cards read
    // identically.
    let label = match name
      .strip_prefix(&want.base)
      .and_then(|rest| rest.strip_prefix('-'))
    {
      Some(suffix) => format!("{}-{suffix}", want.label),
      None => want.label.clone(),
    };
    named.push(Named {
      file: want.file,
      key: want.key.clone(),
      name,
      label,
      pin: want.pin,
    });
  }
  Ok(named)
}

/// Refuse a build in which two files want the same name, listing every clash
/// at once so a large set is fixed in one pass.
fn reject_clashes(wanted: &[Wanted<'_>]) -> Result<()> {
  let mut by_name: BTreeMap<&str, Vec<&Path>> = BTreeMap::new();
  for want in wanted {
    by_name.entry(&want.base).or_default().push(&want.file.path);
  }
  let clashes: Vec<_> = by_name
    .into_iter()
    .filter(|(_, files)| files.len() > 1)
    .collect();
  if clashes.is_empty() {
    return Ok(());
  }

  let mut list = String::new();
  for (name, files) in &clashes {
    list.push_str(&format!("  '{name}'\n"));
    for path in files {
      list.push_str(&format!("      {}\n", path.display()));
    }
  }
  bail!(
    "{} icon name{} claimed by more than one file:\n{}\
         Rename one file in each group, or pass --on-duplicate number to number them.",
    clashes.len(),
    if clashes.len() == 1 { " is" } else { "s are" },
    list
  );
}

/// Whether `recorded` is a name this file could still be given: its own name,
/// or that name with a number added to settle a clash. A file renamed since
/// the last build fails this, and is named from scratch.
fn is_variant_of(recorded: &str, base: &str) -> bool {
  match recorded.strip_prefix(base) {
    Some("") => true,
    Some(rest) => rest
      .strip_prefix('-')
      .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())),
    None => false,
  }
}

/// How the manifest knows an icon: its path inside the icon folder, always
/// forward-slashed so that a manifest written on Windows reads the same
/// everywhere else.
fn manifest_key(group: Option<&str>, path: &Path) -> String {
  let file = path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default();
  match group {
    Some(group) => format!("{group}/{file}"),
    None => file,
  }
}

/// Separate the icons that became real glyphs from the ones that could not.
///
/// A blank or blacked-out glyph is worse than a build failure: it looks like a
/// working icon until someone puts it on a page. Every bad file is reported at
/// once, so a large set is fixed in one pass rather than one file per run.
fn triage(icons: Vec<Icon>, skip_errors: bool) -> Result<Vec<Icon>> {
  let (broken, good): (Vec<_>, Vec<_>) = icons
    .into_iter()
    .partition(|icon| icon.outline.problem.is_some());

  if broken.is_empty() {
    return Ok(good);
  }

  if !skip_errors {
    bail!(
      "{} of {} icons cannot be turned into a glyph:\n{}\n\
             Fix them, or pass --on-error skip to leave them out.",
      broken.len(),
      broken.len() + good.len(),
      problem_list(&broken)
    );
  }

  eprintln!(
    "skipped {} of {} icons:\n{}",
    broken.len(),
    broken.len() + good.len(),
    problem_list(&broken)
  );
  if good.is_empty() {
    bail!("every icon was skipped, so there is nothing to build");
  }
  Ok(good)
}

fn problem_list(broken: &[Icon]) -> String {
  let mut list = String::new();
  for icon in broken {
    let problem = icon.outline.problem.expect("partitioned on being Some");
    list.push_str(&format!(
      "  {} ({})\n      {}\n",
      icon.name,
      icon.source.display(),
      problem.explain()
    ));
  }
  list
}

/// Find a free name for `base`, numbering it `-2`, `-3`, … if it is taken.
fn next_free_name(base: &str, taken: &BTreeMap<String, &Path>) -> String {
  if !taken.contains_key(base) {
    return base.to_string();
  }
  // Starts at 2 so the pair reads as `map-pin` and `map-pin-2`. Skips any
  // number a real file already claimed, so `map-pin-2.svg` keeps its name.
  (2..)
    .map(|suffix| format!("{base}-{suffix}"))
    .find(|candidate| !taken.contains_key(candidate))
    .expect("the range is unbounded, so some candidate is free")
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
      'a'..='z' | '0'..='9' | '-' => name.push(ch),
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
  url
    .replace('%', "%25")
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
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
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
  fn subfolders_prefix_the_name_and_group_the_page() {
    let dir = icon_folder(
      "groups",
      &["check.svg", "arrows/left.svg", "arrows/nested/up.svg"],
    );
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(&files, '\u{e900}', &Manifest::default(), OnDuplicate::Fail).unwrap();

    let named: Vec<_> = icons
      .iter()
      .map(|i| (i.name.as_str(), i.group.as_deref()))
      .collect();
    assert_eq!(
      named,
      [
        // Top-level icons keep a bare name and sort first; nested
        // folders contribute every segment of their path.
        ("check", None),
        ("arrows-left", Some("arrows")),
        ("arrows-nested-up", Some("arrows/nested")),
      ]
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn two_folders_may_hold_the_same_file_name() {
    let dir = icon_folder("same-file", &["arrows/left.svg", "social/left.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(&files, '\u{e900}', &Manifest::default(), OnDuplicate::Fail).unwrap();

    let names: Vec<_> = icons.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["arrows-left", "social-left"]);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn colliding_names_are_numbered_in_sorted_order() {
    // Both reduce to map-pin; the first in sorted order keeps the plain
    // name. '-' sorts before '_', so map-pin.svg wins.
    let dir = icon_folder("collide", &["map-pin.svg", "map_pin.svg", "map pin.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(
      &files,
      '\u{e900}',
      &Manifest::default(),
      OnDuplicate::Number,
    )
    .unwrap();

    let names: Vec<_> = icons.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["map-pin", "map-pin-2", "map-pin-3"]);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn numbering_skips_a_name_a_real_file_already_has() {
    // map-pin-2.svg is a genuine icon, so the numbered fallback must step
    // over it rather than fight it for the name.
    let dir = icon_folder(
      "collide-skip",
      &["map-pin.svg", "map-pin-2.svg", "map_pin.svg"],
    );
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(
      &files,
      '\u{e900}',
      &Manifest::default(),
      OnDuplicate::Number,
    )
    .unwrap();

    let named: Vec<_> = icons
      .iter()
      .map(|i| {
        (
          i.source.file_name().unwrap().to_str().unwrap(),
          i.name.as_str(),
        )
      })
      .collect();
    assert_eq!(
      named,
      [
        ("map-pin-2.svg", "map-pin-2"),
        ("map-pin.svg", "map-pin"),
        ("map_pin.svg", "map-pin-3"),
      ]
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_numbered_icon_gets_a_numbered_label_too() {
    // The preview shows the label, so both cards must not read the same.
    // (`pin_` slugifies to `pin`, and unlike a case variant it survives a
    // case-insensitive filesystem.)
    let dir = icon_folder("collide-label", &["group/pin.svg", "group/pin_.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(
      &files,
      '\u{e900}',
      &Manifest::default(),
      OnDuplicate::Number,
    )
    .unwrap();

    let labels: Vec<_> = icons.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["pin", "pin-2"]);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn adding_an_icon_leaves_existing_codepoints_alone() {
    let dir = icon_folder("stable", &["check.svg", "zoom.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let first = load_icons(&files, '\u{e900}', &Manifest::default(), OnDuplicate::Fail).unwrap();
    let before = codepoints(&first);
    assert_eq!(before["check"], '\u{e900}');
    assert_eq!(before["zoom"], '\u{e901}');

    let mut manifest = Manifest::default();
    for icon in &first {
      manifest.insert(
        &manifest_key(icon.group.as_deref(), &icon.source),
        &icon.name,
        icon.codepoint,
      );
    }

    // This one sorts before both existing icons, so without the manifest it
    // would take U+E900 and shift everything after it.
    std::fs::write(dir.join("aaa.svg"), SQUARE).unwrap();
    let files = collect_svgs(&dir).unwrap();
    let second = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Fail).unwrap();
    let after = codepoints(&second);

    assert_eq!(after["check"], '\u{e900}');
    assert_eq!(after["zoom"], '\u{e901}');
    assert_eq!(after["aaa"], '\u{e902}');
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_deleted_icons_codepoint_is_never_handed_to_another_icon() {
    let mut manifest = Manifest::default();
    manifest.insert("retired.svg", "retired", '\u{e900}');

    let dir = icon_folder("retired", &["fresh.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Fail).unwrap();

    assert_eq!(icons[0].name, "fresh");
    assert_eq!(icons[0].codepoint, '\u{e901}', "U+E900 is still reserved");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn clashing_names_stop_the_build() {
    // A number is not part of either file, so which file gets which depends on
    // what else is in the folder. Naming both files is the fix that stays
    // fixed, and the build asks for it rather than guessing.
    let dir = icon_folder("clash-fails", &["map-pin.svg", "map_pin.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let error = load_icons(&files, '\u{e900}', &Manifest::default(), OnDuplicate::Fail)
      .unwrap_err()
      .to_string();

    assert!(error.contains("'map-pin'"), "{error}");
    assert!(error.contains("map-pin.svg"), "{error}");
    assert!(error.contains("map_pin.svg"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_numbered_name_stays_with_the_file_that_earned_it() {
    // Numbering used to fall out of sort order alone, so a third clashing file
    // that sorted first took `map-pin` and pushed every later one along —
    // renaming icons that pages already used, and moving their codepoints with
    // them. The manifest records which file was given which number, so the
    // newcomer takes the next one instead.
    let dir = icon_folder("renumber", &["map-pin.svg", "map_pin.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let first = load_icons(
      &files,
      '\u{e900}',
      &Manifest::default(),
      OnDuplicate::Number,
    )
    .unwrap();

    let mut manifest = Manifest::default();
    for icon in &first {
      manifest.insert(
        &manifest_key(icon.group.as_deref(), &icon.source),
        &icon.name,
        icon.codepoint,
      );
    }

    // A space sorts before both '-' and '_', so this one is walked first.
    std::fs::write(dir.join("map pin.svg"), SQUARE).unwrap();
    let files = collect_svgs(&dir).unwrap();
    let second = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Number).unwrap();

    let named: Vec<_> = second
      .iter()
      .map(|i| {
        (
          i.source.file_name().unwrap().to_str().unwrap(),
          i.name.as_str(),
          i.codepoint,
        )
      })
      .collect();
    assert_eq!(
      named,
      [
        ("map pin.svg", "map-pin-3", '\u{e902}'),
        ("map-pin.svg", "map-pin", '\u{e900}'),
        ("map_pin.svg", "map-pin-2", '\u{e901}'),
      ]
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_manifest_from_an_older_icofon_keeps_its_codepoints() {
    // 0.3 and earlier keyed records by name. Reading them is what stops the
    // first build after an upgrade from reassigning every codepoint in the set.
    let dir = icon_folder("legacy", &["check.svg", "aaa.svg"]);
    let path = dir.join(manifest::DEFAULT_FILE);
    std::fs::write(&path, r#"{ "codepoints": { "check": "e900" } }"#).unwrap();

    let manifest = Manifest::load(&path).unwrap();
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Fail).unwrap();
    let after = codepoints(&icons);

    assert_eq!(after["check"], '\u{e900}');
    assert_eq!(after["aaa"], '\u{e901}', "the free slot, not check's");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_pin_moves_the_icon_it_names() {
    // Renaming heart.svg to uE9F0-heart.svg is a deliberate instruction to
    // move that icon, so the pin decides where it lands whatever the manifest
    // recorded for the file it used to be.
    let mut manifest = Manifest::default();
    manifest.insert("heart.svg", "heart", '\u{e900}');

    let dir = icon_folder("pin-moves", &["uE9F0-heart.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let icons = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Fail).unwrap();

    assert_eq!(icons[0].name, "heart");
    assert_eq!(icons[0].codepoint, '\u{e9f0}');
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_pin_cannot_steal_another_icons_codepoint() {
    // U+E900 already belongs to heart, so letting badge pin it would leave
    // two icons fighting over one codepoint.
    let mut manifest = Manifest::default();
    manifest.insert("heart.svg", "heart", '\u{e900}');

    let dir = icon_folder("pin-steals", &["uE900-badge.svg", "heart.svg"]);
    let files = collect_svgs(&dir).unwrap();
    let error = load_icons(&files, '\u{e900}', &manifest, OnDuplicate::Fail)
      .unwrap_err()
      .to_string();
    assert!(error.contains("pinned by a file name"), "{error}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn explicit_codepoints_are_recognized() {
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
    // Underscores are separators too, so names are consistently hyphenated.
    assert_eq!(sanitize_name("zoom_in (2)"), "zoom-in-2");
    assert_eq!(
      sanitize_name("empty_states/no_results"),
      "empty-states-no-results"
    );
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
