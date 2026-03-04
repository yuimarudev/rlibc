use super::analysis::{AnalysisResult, StyleViolation};
use super::text::{
  TextEdit, apply_edits, build_line_starts, detect_newline_sequence, has_only_whitespace,
  line_start_offset,
};
use syn::spanned::Spanned;
use syn::{File, Item};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemGroup {
  Mod,
  Use,
  ConstOrStatic,
  TypeDefinition,
  Impl,
  Function,
  Other,
}

#[derive(Clone, Debug)]
struct ItemDescriptor {
  group: ItemGroup,
  start_line: usize,
  end_line: usize,
}

impl ItemGroup {
  const fn label(self) -> &'static str {
    match self {
      Self::Mod => "mod",
      Self::Use => "use",
      Self::ConstOrStatic => "const/static",
      Self::TypeDefinition => "type-definition",
      Self::Impl => "impl",
      Self::Function => "function",
      Self::Other => "other",
    }
  }
}

/// Analyzes top-level layout and returns violations plus optional fixed source.
///
/// # Errors
///
/// Returns an error when the input cannot be parsed as Rust code or when span
/// line information is unavailable.
pub fn analyze_top_level_layout(source: &str) -> Result<AnalysisResult, syn::Error> {
  let parsed = syn::parse_file(source)?;

  analyze_parsed_file(source, &parsed)
}

fn analyze_parsed_file(source: &str, parsed: &File) -> Result<AnalysisResult, syn::Error> {
  let descriptors = parsed
    .items
    .iter()
    .map(build_descriptor)
    .collect::<Result<Vec<_>, _>>()?;

  if descriptors.len() <= 1 {
    return Ok(AnalysisResult {
      violations: Vec::new(),
      formatted_source: None,
    });
  }

  let line_starts = build_line_starts(source);
  let newline = detect_newline_sequence(source);
  let mut edits = Vec::new();
  let mut violations = Vec::new();

  for pair in descriptors.windows(2) {
    let [current, next] = pair else {
      continue;
    };

    if next.start_line <= current.end_line {
      continue;
    }

    let expected = expected_blank_lines(current.group, next.group);
    let actual = next.start_line.saturating_sub(current.end_line + 1);
    let gap_start_line = current.end_line + 1;
    let gap_end_line = next.start_line;
    let start_byte = line_start_offset(line_starts.as_slice(), gap_start_line);
    let end_byte = line_start_offset(line_starts.as_slice(), gap_end_line);

    if !has_only_whitespace(source, start_byte, end_byte) {
      continue;
    }

    if actual == expected {
      continue;
    }

    let message = format!(
      "top-level spacing between {} and {} must be {expected} blank line(s), found {actual}",
      current.group.label(),
      next.group.label(),
    );

    violations.push(StyleViolation {
      line: next.start_line,
      column: 1,
      code: "CSTYLE001",
      message,
    });

    edits.push(TextEdit {
      start_byte,
      end_byte,
      replacement: newline.repeat(expected),
    });
  }

  let formatted_source = apply_edits(source, &mut edits);

  Ok(AnalysisResult {
    violations,
    formatted_source,
  })
}

fn build_descriptor(item: &Item) -> Result<ItemDescriptor, syn::Error> {
  let span = item.span();
  let start = span.start();
  let end = span.end();

  if start.line == 0 || end.line == 0 {
    return Err(syn::Error::new(
      span,
      "span line information is unavailable; enable proc-macro2 span-locations",
    ));
  }

  Ok(ItemDescriptor {
    group: classify_item(item),
    start_line: start.line,
    end_line: end.line,
  })
}

const fn classify_item(item: &Item) -> ItemGroup {
  match item {
    Item::Mod(_) => ItemGroup::Mod,
    Item::Use(_) => ItemGroup::Use,
    Item::Const(_) | Item::Static(_) => ItemGroup::ConstOrStatic,
    Item::Struct(_)
    | Item::Enum(_)
    | Item::Trait(_)
    | Item::Type(_)
    | Item::Union(_)
    | Item::TraitAlias(_) => ItemGroup::TypeDefinition,
    Item::Impl(_) => ItemGroup::Impl,
    Item::Fn(_) => ItemGroup::Function,
    _ => ItemGroup::Other,
  }
}

fn expected_blank_lines(current: ItemGroup, next: ItemGroup) -> usize {
  if current == next {
    match current {
      ItemGroup::Mod | ItemGroup::Use | ItemGroup::ConstOrStatic => 0,
      ItemGroup::TypeDefinition | ItemGroup::Impl | ItemGroup::Function | ItemGroup::Other => 1,
    }
  } else {
    1
  }
}

#[cfg(test)]
mod tests {
  use super::analyze_top_level_layout;

  #[test]
  fn keeps_mod_items_without_blank_line() {
    let source = "mod a;\n\nmod b;\n";
    let result = analyze_top_level_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.formatted_source, Some("mod a;\nmod b;\n".to_owned()));
  }

  #[test]
  fn inserts_blank_line_between_structs() {
    let source = "struct A;\nstruct B;\n";
    let result = analyze_top_level_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("struct A;\n\nstruct B;\n".to_owned())
    );
  }

  #[test]
  fn inserts_blank_line_between_different_groups() {
    let source = "use std::fmt;\nfn run() {}\n";
    let result = analyze_top_level_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("use std::fmt;\n\nfn run() {}\n".to_owned())
    );
  }

  #[test]
  fn keeps_comment_gap_untouched() {
    let source = "mod a;\n// comment\nmod b;\n";
    let result = analyze_top_level_layout(source).expect("parse should succeed");

    assert!(result.violations.is_empty());
    assert!(result.formatted_source.is_none());
  }

  #[test]
  fn preserves_crlf_newlines() {
    let source = "mod a;\r\n\r\nmod b;\r\n";
    let result = analyze_top_level_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("mod a;\r\nmod b;\r\n".to_owned())
    );
  }
}
