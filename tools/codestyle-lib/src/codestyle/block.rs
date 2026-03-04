use super::analysis::{AnalysisResult, StyleViolation};
use super::text::{
  TextEdit, apply_edits, build_line_starts, detect_newline_sequence, has_only_whitespace,
  line_start_offset,
};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Block, Expr, File, ImplItemFn, ItemFn, Stmt, TraitItemFn};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatementGroup {
  Declaration,
  Control,
  ReturnTerminal,
  Other,
}

#[derive(Clone, Debug)]
struct StatementDescriptor {
  group: StatementGroup,
  start_line: usize,
  end_line: usize,
}

struct BlockCollector<'ast> {
  blocks: Vec<&'ast Block>,
  function_depth: usize,
}

struct ReturnFinder {
  found: bool,
}

impl StatementGroup {
  const fn label(self) -> &'static str {
    match self {
      Self::Declaration => "declaration",
      Self::Control => "control-statement",
      Self::ReturnTerminal => "terminal-return",
      Self::Other => "statement",
    }
  }
}

impl<'ast> Visit<'ast> for BlockCollector<'ast> {
  fn visit_item_fn(&mut self, node: &'ast ItemFn) {
    self.function_depth = self.function_depth.saturating_add(1);
    visit::visit_item_fn(self, node);
    self.function_depth = self.function_depth.saturating_sub(1);
  }

  fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
    self.function_depth = self.function_depth.saturating_add(1);
    visit::visit_impl_item_fn(self, node);
    self.function_depth = self.function_depth.saturating_sub(1);
  }

  fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
    self.function_depth = self.function_depth.saturating_add(1);
    visit::visit_trait_item_fn(self, node);
    self.function_depth = self.function_depth.saturating_sub(1);
  }

  fn visit_block(&mut self, node: &'ast Block) {
    if self.function_depth > 0 {
      self.blocks.push(node);
    }

    visit::visit_block(self, node);
  }
}

impl<'ast> Visit<'ast> for ReturnFinder {
  fn visit_expr_return(&mut self, _node: &'ast syn::ExprReturn) {
    self.found = true;
  }

  fn visit_expr(&mut self, node: &'ast Expr) {
    if self.found {
      return;
    }

    visit::visit_expr(self, node);
  }
}

/// Analyzes non-top-level layout rules inside functions/impls and returns
/// violations plus optional fixed source.
///
/// # Errors
///
/// Returns an error when the input cannot be parsed as Rust code or when span
/// line information is unavailable.
pub fn analyze_block_layout(source: &str) -> Result<AnalysisResult, syn::Error> {
  let parsed = syn::parse_file(source)?;

  analyze_parsed_file(source, &parsed)
}

fn analyze_parsed_file(source: &str, parsed: &File) -> Result<AnalysisResult, syn::Error> {
  let mut collector = BlockCollector {
    blocks: Vec::new(),
    function_depth: 0,
  };

  collector.visit_file(parsed);

  if collector.blocks.is_empty() {
    return Ok(AnalysisResult {
      violations: Vec::new(),
      formatted_source: None,
    });
  }

  let line_starts = build_line_starts(source);
  let newline = detect_newline_sequence(source);
  let mut edits = Vec::new();
  let mut violations = Vec::new();

  for block in collector.blocks {
    analyze_single_block(
      source,
      block,
      line_starts.as_slice(),
      newline,
      &mut edits,
      &mut violations,
    )?;
  }

  let formatted_source = apply_edits(source, &mut edits);

  Ok(AnalysisResult {
    violations,
    formatted_source,
  })
}

fn analyze_single_block(
  source: &str,
  block: &Block,
  line_starts: &[usize],
  newline: &str,
  edits: &mut Vec<TextEdit>,
  violations: &mut Vec<StyleViolation>,
) -> Result<(), syn::Error> {
  if block.stmts.len() <= 1 {
    return Ok(());
  }

  let descriptors = block
    .stmts
    .iter()
    .enumerate()
    .map(|(index, statement)| {
      let is_final_statement = index + 1 == block.stmts.len();

      build_statement_descriptor(statement, is_final_statement)
    })
    .collect::<Result<Vec<_>, _>>()?;

  for pair in descriptors.windows(2) {
    let [current, next] = pair else {
      continue;
    };

    if next.start_line <= current.end_line {
      continue;
    }

    let Some(expected) = expected_blank_lines(current.group, next.group) else {
      continue;
    };
    let actual = next.start_line.saturating_sub(current.end_line + 1);

    if actual == expected {
      continue;
    }

    let gap_start_line = current.end_line + 1;
    let gap_end_line = next.start_line;
    let start_byte = line_start_offset(line_starts, gap_start_line);
    let end_byte = line_start_offset(line_starts, gap_end_line);

    if !has_only_whitespace(source, start_byte, end_byte) {
      continue;
    }

    violations.push(StyleViolation {
      line: next.start_line,
      column: 1,
      code: "CSTYLE101",
      message: format!(
        "block spacing between {} and {} must be {expected} blank line(s), found {actual}",
        current.group.label(),
        next.group.label(),
      ),
    });

    edits.push(TextEdit {
      start_byte,
      end_byte,
      replacement: newline.repeat(expected),
    });
  }

  Ok(())
}

fn build_statement_descriptor(
  statement: &Stmt,
  is_final_statement: bool,
) -> Result<StatementDescriptor, syn::Error> {
  let span = statement.span();
  let start = span.start();
  let end = span.end();

  if start.line == 0 || end.line == 0 {
    return Err(syn::Error::new(
      span,
      "span line information is unavailable; enable proc-macro2 span-locations",
    ));
  }

  Ok(StatementDescriptor {
    group: classify_statement(statement, is_final_statement),
    start_line: start.line,
    end_line: end.line,
  })
}

fn classify_statement(statement: &Stmt, is_final_statement: bool) -> StatementGroup {
  if is_declaration_statement(statement) {
    return StatementGroup::Declaration;
  }

  if is_final_statement && statement_contains_return(statement) {
    return StatementGroup::ReturnTerminal;
  }

  if is_control_statement(statement) {
    return StatementGroup::Control;
  }

  StatementGroup::Other
}

const fn expected_blank_lines(current: StatementGroup, next: StatementGroup) -> Option<usize> {
  match (current, next) {
    (StatementGroup::Declaration, StatementGroup::Declaration) => Some(0),
    (StatementGroup::Other, StatementGroup::Other) => None,
    _ => Some(1),
  }
}

const fn is_declaration_statement(statement: &Stmt) -> bool {
  matches!(statement, Stmt::Local(_))
    || matches!(
      statement,
      Stmt::Item(syn::Item::Const(_) | syn::Item::Static(_))
    )
}

const fn is_control_statement(statement: &Stmt) -> bool {
  match statement {
    Stmt::Expr(expression, _) => is_control_expression(expression),
    Stmt::Local(_) | Stmt::Item(_) | Stmt::Macro(_) => false,
  }
}

const fn is_control_expression(expression: &Expr) -> bool {
  matches!(
    expression,
    Expr::If(_)
      | Expr::Match(_)
      | Expr::Loop(_)
      | Expr::ForLoop(_)
      | Expr::While(_)
      | Expr::Block(_)
  )
}

fn statement_contains_return(statement: &Stmt) -> bool {
  let mut finder = ReturnFinder { found: false };

  finder.visit_stmt(statement);

  finder.found
}

#[cfg(test)]
mod tests {
  use super::analyze_block_layout;

  #[test]
  fn declaration_group_cannot_have_blank_lines() {
    let source = "fn f() {\n  let a = 1;\n\n  let b = 2;\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("fn f() {\n  let a = 1;\n  let b = 2;\n}\n".to_owned())
    );
  }

  #[test]
  fn declaration_and_processing_groups_need_separator() {
    let source = "fn f() {\n  let a = 1;\n  call(a);\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("fn f() {\n  let a = 1;\n\n  call(a);\n}\n".to_owned())
    );
  }

  #[test]
  fn control_statements_are_separated() {
    let source = "fn f(flag: bool) {\n  if flag {}\n  while false {}\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("fn f(flag: bool) {\n  if flag {}\n\n  while false {}\n}\n".to_owned())
    );
  }

  #[test]
  fn terminal_return_is_its_own_group() {
    let source = "fn f() -> i32 {\n  let a = 1;\n  return a;\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("fn f() -> i32 {\n  let a = 1;\n\n  return a;\n}\n".to_owned())
    );
  }

  #[test]
  fn skips_gaps_with_comments() {
    let source = "fn f() {\n  let a = 1;\n  // keep\n  call(a);\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert!(result.violations.is_empty());
    assert!(result.formatted_source.is_none());
  }

  #[test]
  fn applies_rules_to_nested_blocks() {
    let source = "fn f(flag: bool) {\n  if flag {\n    let a = 1;\n    call(a);\n  }\n}\n";
    let result = analyze_block_layout(source).expect("parse should succeed");

    assert_eq!(result.violations.len(), 1);
    assert_eq!(
      result.formatted_source,
      Some("fn f(flag: bool) {\n  if flag {\n    let a = 1;\n\n    call(a);\n  }\n}\n".to_owned())
    );
  }
}
