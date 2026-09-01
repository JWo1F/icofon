//! A small CSS writer.
//!
//! Rules are built from [`Selector`] and [`Value`] rather than glued together
//! as strings. Everything that reaches the page goes through a constructor
//! that knows how to escape it, so a font family carrying a quote ends up as
//! text inside a declaration instead of ending the declaration and opening a
//! rule of its own.

use std::fmt::{self, Write as _};

/// A CSS value, already written in the form it will appear.
#[derive(Clone)]
pub struct Value(String);

impl Value {
  /// A bare keyword, such as `normal` or `block`.
  pub fn keyword(word: &str) -> Value {
    Value(word.to_string())
  }

  /// A quoted string. Quotes and backslashes inside `text` are escaped, so it
  /// cannot end the string early.
  pub fn quoted(text: &str) -> Value {
    Value(format!("'{}'", escape_string(text, '\'')))
  }

  /// `url('...')`, escaped like any other quoted string.
  pub fn url(url: &str) -> Value {
    Value(format!("url('{}')", escape_string(url, '\'')))
  }

  /// A function call such as `format('truetype')`.
  pub fn call(name: &str, argument: Value) -> Value {
    Value(format!("{name}({})", argument.0))
  }

  /// The character a `content` property should render, as a CSS escape.
  pub fn glyph(c: char) -> Value {
    Value(format!("\"\\{:x}\"", c as u32))
  }

  /// Several values separated by spaces, as one `src` entry takes them.
  pub fn list(parts: impl IntoIterator<Item = Value>) -> Value {
    let parts: Vec<String> = parts.into_iter().map(|part| part.0).collect();
    Value(parts.join(" "))
  }

  /// Alternatives separated by commas, as `src` takes a list of font files.
  /// Each goes on its own line, since one per line is how these are read.
  pub fn comma_list(parts: impl IntoIterator<Item = Value>) -> Value {
    let parts: Vec<String> = parts.into_iter().map(|part| part.0).collect();
    Value(parts.join(",\n       "))
  }
}

/// A CSS selector, built from parts that are escaped as they are added.
#[derive(Clone)]
pub struct Selector(String);

impl Selector {
  /// A class selector: `.icon`.
  pub fn class(name: &str) -> Selector {
    Selector(format!(".{}", escape_ident(name)))
  }

  /// `[attr^="value"]` — an attribute starting with `value`.
  pub fn attribute_starts_with(attribute: &str, value: &str) -> Selector {
    Selector(format!(
      "[{}^=\"{}\"]",
      escape_ident(attribute),
      escape_string(value, '"')
    ))
  }

  /// `[attr*="value"]` — an attribute containing `value`.
  pub fn attribute_contains(attribute: &str, value: &str) -> Selector {
    Selector(format!(
      "[{}*=\"{}\"]",
      escape_ident(attribute),
      escape_string(value, '"')
    ))
  }

  /// Both selectors on the same element: `.icon.icon-arrow-left`.
  pub fn and(self, other: Selector) -> Selector {
    Selector(format!("{}{}", self.0, other.0))
  }

  /// The element's `::before` pseudo-element.
  pub fn before(self) -> Selector {
    Selector(format!("{}::before", self.0))
  }
}

impl fmt::Display for Selector {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

/// A stylesheet under construction.
#[derive(Default)]
pub struct Sheet {
  out: String,
}

impl Sheet {
  pub fn new() -> Sheet {
    Sheet::default()
  }

  /// An at-rule with a body, such as `@font-face`.
  pub fn at_rule(&mut self, name: &str, body: impl FnOnce(&mut Block)) {
    self.write_block(&format!("@{name}"), body);
  }

  /// A rule. Several selectors are written one per line, as CSS allows.
  pub fn rule(
    &mut self,
    selectors: impl IntoIterator<Item = Selector>,
    body: impl FnOnce(&mut Block),
  ) {
    let selectors: Vec<String> = selectors.into_iter().map(|selector| selector.0).collect();
    self.write_block(&selectors.join(",\n"), body);
  }

  /// A blank line, separating one group of rules from the next.
  pub fn blank_line(&mut self) {
    self.out.push('\n');
  }

  pub fn finish(self) -> String {
    self.out
  }

  fn write_block(&mut self, head: &str, body: impl FnOnce(&mut Block)) {
    let mut block = Block { out: String::new() };
    body(&mut block);
    let _ = write!(self.out, "{head} {{\n{}}}\n", block.out);
  }
}

/// The inside of a rule: its declarations.
pub struct Block {
  out: String,
}

impl Block {
  /// `property: value;`
  pub fn set(&mut self, property: &str, value: Value) {
    let _ = writeln!(self.out, "  {property}: {};", value.0);
  }

  /// `property: value !important;`
  pub fn set_important(&mut self, property: &str, value: Value) {
    let _ = writeln!(self.out, "  {property}: {} !important;", value.0);
  }
}

/// Escape a string so it cannot end the quotes it sits in.
fn escape_string(text: &str, quote: char) -> String {
  let mut out = String::with_capacity(text.len());
  for c in text.chars() {
    match c {
      '\\' => out.push_str("\\\\"),
      // A newline cannot appear literally inside a CSS string.
      '\n' => out.push_str("\\A "),
      c if c == quote => {
        out.push('\\');
        out.push(c);
      }
      c => out.push(c),
    }
  }
  out
}

/// Escape an identifier — a class or attribute name — so that punctuation in it
/// is read as part of the name rather than as selector syntax.
fn escape_ident(ident: &str) -> String {
  let mut out = String::with_capacity(ident.len());
  for (i, c) in ident.chars().enumerate() {
    match c {
      'a'..='z' | 'A'..='Z' | '_' | '-' => out.push(c),
      // A leading digit would be read as the start of a number, so it is
      // written as a hex escape with the space that terminates one.
      '0'..='9' if i == 0 => {
        let _ = write!(out, "\\3{c} ");
      }
      '0'..='9' => out.push(c),
      c => {
        out.push('\\');
        out.push(c);
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_quote_in_a_value_is_escaped_rather_than_ending_the_string() {
    assert_eq!(Value::quoted("it's").0, r"'it\'s'");
    assert_eq!(Value::url("a'b.ttf").0, r"url('a\'b.ttf')");
    // A backslash of its own would otherwise escape the closing quote.
    assert_eq!(Value::quoted(r"back\slash").0, r"'back\\slash'");
  }

  #[test]
  fn punctuation_in_a_class_name_is_escaped_rather_than_read_as_syntax() {
    // Without escaping this would select `.a` inside `.b`, not a class named
    // `a.b`, and the trailing part would silently select nothing.
    assert_eq!(Selector::class("a.b").to_string(), r".a\.b");
    assert_eq!(Selector::class("with space").to_string(), r".with\ space");
    // A name that starts with a digit is not a valid identifier as written.
    assert_eq!(Selector::class("2x").to_string(), r".\32 x");
    // The ordinary case is left completely alone.
    assert_eq!(
      Selector::class("icon-arrow-left").to_string(),
      ".icon-arrow-left"
    );
  }

  #[test]
  fn a_sheet_writes_the_rules_it_is_given() {
    let mut sheet = Sheet::new();
    sheet.rule([Selector::class("icon").before()], |block| {
      block.set("content", Value::glyph('\u{e900}'));
      block.set_important("font-family", Value::quoted("My Icons"));
    });

    assert_eq!(
      sheet.finish(),
      ".icon::before {\n  content: \"\\e900\";\n  font-family: 'My Icons' !important;\n}\n"
    );
  }
}
