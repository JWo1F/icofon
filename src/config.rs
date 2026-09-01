//! Where a build's settings come from.
//!
//! Three layers, each overriding the one before it: the built-in defaults, an
//! `icofon.toml` found by walking up from the working directory, and the flags
//! on the command line. A project checks the file in and stops retyping the
//! command; a one-off run needs no file at all.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The name of the file looked for on the way up to the root.
pub const FILE: &str = "icofon.toml";

/// A font container to write. Ordered by how much a browser wants it.
#[derive(
  Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum Format {
  /// Brotli-compressed, and the smallest by a wide margin. Every browser since
  /// 2016 takes it.
  Woff2,
  /// Deflate-compressed. The fallback for anything older.
  Woff,
  /// Uncompressed TrueType, which is what desktop applications and non-web
  /// tooling expect.
  Ttf,
}

impl Format {
  pub fn extension(self) -> &'static str {
    match self {
      Format::Woff2 => "woff2",
      Format::Woff => "woff",
      Format::Ttf => "ttf",
    }
  }

  /// The name CSS knows the container by, which is not always its extension.
  pub fn css_format(self) -> &'static str {
    match self {
      Format::Woff2 => "woff2",
      Format::Woff => "woff",
      Format::Ttf => "truetype",
    }
  }
}

/// What to do about an icon that cannot be turned into a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum OnError {
  /// Name every bad file and build nothing.
  Fail,
  /// Leave them out, and list them on stderr.
  Skip,
}

/// `icofon.toml`, every field optional so a file can set only what it cares
/// about.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct File {
  pub source: Option<PathBuf>,
  pub out: Option<PathBuf>,
  pub name: Option<String>,
  pub formats: Option<Vec<Format>>,
  pub prefix: Option<String>,
  pub base_class: Option<bool>,
  pub preview: Option<bool>,
  pub manifest: Option<bool>,
  pub manifest_path: Option<PathBuf>,
  pub start: Option<String>,
  pub on_error: Option<OnError>,
}

impl File {
  /// Look for `icofon.toml` in `from` and every directory above it.
  ///
  /// Returns the file and where it was found, so paths inside it can be read
  /// relative to it rather than to wherever the command was run.
  pub fn discover(from: &Path) -> Result<Option<(File, PathBuf)>> {
    for dir in from.ancestors() {
      let path = dir.join(FILE);
      if path.is_file() {
        let file = File::load(&path)?;
        return Ok(Some((file, dir.to_path_buf())));
      }
    }
    Ok(None)
  }

  pub fn load(path: &Path) -> Result<File> {
    let text =
      std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))
  }
}

/// The settings a build actually runs with, once the layers are merged.
#[derive(Debug, Clone)]
pub struct Build {
  pub source: PathBuf,
  pub out: PathBuf,
  pub name: String,
  pub formats: Vec<Format>,
  pub prefix: String,
  pub base_class: bool,
  pub preview: bool,
  /// Where to keep codepoints, or `None` to assign them fresh every build.
  pub manifest: Option<PathBuf>,
  pub start: char,
  pub on_error: OnError,
}

impl Build {
  /// The path of one of the font files this build writes.
  pub fn font_path(&self, format: Format) -> PathBuf {
    self
      .out
      .join(format!("{}.{}", self.name, format.extension()))
  }

  pub fn css_path(&self) -> PathBuf {
    self.out.join(format!("{}.css", self.name))
  }

  pub fn preview_path(&self) -> PathBuf {
    self.out.join("index.html")
  }

  /// Formats in the order a browser should be offered them, smallest first,
  /// regardless of the order they were requested in.
  pub fn ordered_formats(&self) -> Vec<Format> {
    let mut formats = self.formats.clone();
    formats.sort();
    formats.dedup();
    formats
  }
}
