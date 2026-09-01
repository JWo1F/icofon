//! The stylesheet that makes the generated font usable from HTML.

use crate::font::Icon;

/// How an icon is named in HTML, and so how the stylesheet selects it.
///
/// By default the stylesheet matches any class starting with the prefix, so an
/// icon is a single `icon-arrow-left` class. That is convenient, but it claims
/// the whole `icon-*` namespace: a class of your own that happens to start the
/// same way picks up the icon font too.
///
/// With `base_class` the prefix becomes a class in its own right and an icon is
/// written `class="icon icon-arrow-left"`. Every rule is then scoped to
/// elements carrying both, so nothing outside this font is touched.
#[derive(Clone, Copy)]
pub struct Classes<'a> {
  pub prefix: &'a str,
  pub base_class: bool,
}

impl Classes<'_> {
  /// What to write in a `class` attribute for the icon named `name`.
  pub fn attr(&self, name: &str) -> String {
    let prefix = self.prefix;
    if self.base_class {
      format!("{prefix} {prefix}-{name}")
    } else {
      format!("{prefix}-{name}")
    }
  }

  /// The selector matching an element that carries [`Classes::attr`].
  pub fn selector(&self, name: &str) -> String {
    let prefix = self.prefix;
    if self.base_class {
      format!(".{prefix}.{prefix}-{name}")
    } else {
      format!(".{prefix}-{name}")
    }
  }
}

/// Render the stylesheet for `icons`.
///
/// `font_url` is what goes into `src: url(...)`, and is normally just the font
/// file's name so that the CSS works wherever the two files are copied to
/// together.
pub fn render(icons: &[Icon], family: &str, classes: Classes<'_>, font_url: &str) -> String {
  let prefix = classes.prefix;
  let mut css = String::new();

  css.push_str(&format!(
    "@font-face {{\n  \
           font-family: '{family}';\n  \
           src: url('{font_url}') format('truetype');\n  \
           font-weight: normal;\n  \
           font-style: normal;\n  \
           font-display: block;\n\
         }}\n\n"
  ));

  // With a base class there is one class to hang the font rules on. Without
  // one, match both `class="icon-foo"` and `class="btn icon-foo"` so the
  // rules apply without having to also write the bare `.icon` class.
  let base_selector = if classes.base_class {
    format!(".{prefix}")
  } else {
    format!("[class^=\"{prefix}-\"],\n[class*=\" {prefix}-\"]")
  };
  css.push_str(&format!(
    "{base_selector} {{\n  \
           font-family: '{family}' !important;\n  \
           font-style: normal;\n  \
           font-weight: normal;\n  \
           font-variant: normal;\n  \
           text-transform: none;\n  \
           line-height: 1;\n  \
           speak: never;\n  \
           -webkit-font-smoothing: antialiased;\n  \
           -moz-osx-font-smoothing: grayscale;\n\
         }}\n\n"
  ));

  for icon in icons {
    css.push_str(&format!(
      "{selector}::before {{\n  content: \"\\{code:x}\";\n}}\n",
      selector = classes.selector(&icon.name),
      code = icon.codepoint as u32,
    ));
  }

  css
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::svg;

  fn icon(name: &str, codepoint: char) -> Icon {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                        <rect width="24" height="24" fill="#000"/>
                      </svg>"##;
    Icon {
      name: name.to_string(),
      label: name.to_string(),
      group: None,
      source: name.into(),
      codepoint,
      outline: svg::parse(svg.as_bytes(), name).unwrap(),
    }
  }

  #[test]
  fn by_default_one_class_carries_both_the_font_and_the_glyph() {
    let classes = Classes {
      prefix: "icon",
      base_class: false,
    };
    let css = render(&[icon("arrow-left", '\u{e900}')], "Icons", classes, "f.ttf");

    assert!(css.contains(r#"[class^="icon-"]"#));
    assert!(css.contains(r#"[class*=" icon-"]"#));
    assert!(css.contains(".icon-arrow-left::before"));
  }

  #[test]
  fn a_base_class_scopes_every_rule_to_icons_from_this_font() {
    let classes = Classes {
      prefix: "icon",
      base_class: true,
    };
    let css = render(&[icon("arrow-left", '\u{e900}')], "Icons", classes, "f.ttf");

    // Nothing is claimed by name alone, so a class of the user's own that
    // happens to start `icon-` is left entirely untouched.
    assert!(!css.contains("[class"));
    assert!(css.contains(".icon {\n"));
    assert!(css.contains(".icon.icon-arrow-left::before"));
  }

  #[test]
  fn markup_and_selector_agree_in_both_modes() {
    let bare = Classes {
      prefix: "ico",
      base_class: false,
    };
    assert_eq!(bare.attr("star"), "ico-star");
    assert_eq!(bare.selector("star"), ".ico-star");

    let based = Classes {
      prefix: "ico",
      base_class: true,
    };
    assert_eq!(based.attr("star"), "ico ico-star");
    assert_eq!(based.selector("star"), ".ico.ico-star");
  }
}
