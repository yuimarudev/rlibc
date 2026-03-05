use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn include_root() -> PathBuf {
  repo_root().join("include")
}

fn read_text(path: &Path) -> String {
  std::fs::read_to_string(path)
    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn read_header(header_name: &str) -> String {
  read_text(&include_root().join(header_name))
}

fn collect_header_files(dir: &Path, files: &mut Vec<PathBuf>) {
  let entries = std::fs::read_dir(dir)
    .unwrap_or_else(|error| panic!("failed to list {}: {error}", dir.display()));

  for entry in entries {
    let entry = entry
      .unwrap_or_else(|error| panic!("failed to read dir entry in {}: {error}", dir.display()));
    let path = entry.path();

    if path.is_dir() {
      collect_header_files(&path, files);
      continue;
    }

    if path.extension().is_some_and(|ext| ext == "h") {
      files.push(path);
    }
  }
}

fn all_header_text() -> String {
  let mut header_files = Vec::new();

  collect_header_files(&include_root(), &mut header_files);
  header_files.sort();

  header_files
    .into_iter()
    .map(|path| read_text(&path))
    .collect::<Vec<_>>()
    .join("\n")
}

fn all_header_paths_relative() -> Vec<String> {
  let include = include_root();
  let mut header_files = Vec::new();

  collect_header_files(&include, &mut header_files);
  header_files.sort();

  header_files
    .into_iter()
    .map(|path| {
      path
        .strip_prefix(&include)
        .unwrap_or_else(|error| panic!("failed to relativize {}: {error}", path.display()))
        .to_string_lossy()
        .replace('\\', "/")
    })
    .collect()
}

fn find_c_compiler() -> Option<String> {
  let mut candidates = Vec::new();

  if let Some(cc_from_env) = std::env::var_os("CC") {
    let cc_from_env = cc_from_env.to_string_lossy().trim().to_string();

    if !cc_from_env.is_empty() {
      candidates.push(cc_from_env);
    }
  }

  for candidate in ["cc", "clang", "gcc"] {
    candidates.push(candidate.to_string());
  }

  candidates.into_iter().find(|candidate| {
    Command::new(candidate)
      .arg("--version")
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status()
      .is_ok()
  })
}

fn exported_symbols_from_map() -> Vec<String> {
  let map_text = read_text(&repo_root().join("rlibc.map"));
  let mut in_global_block = false;
  let mut symbols = Vec::new();

  for line in map_text.lines() {
    let trimmed = line.trim();

    if trimmed == "global:" {
      in_global_block = true;
      continue;
    }

    if trimmed == "local:" {
      break;
    }

    if !in_global_block {
      continue;
    }

    let Some(symbol) = trimmed.strip_suffix(';') else {
      continue;
    };
    let symbol = symbol.trim();

    if symbol.is_empty() || symbol == "*" {
      continue;
    }

    symbols.push(symbol.to_string());
  }

  symbols
}

const fn is_identifier_byte(byte: u8) -> bool {
  byte.is_ascii_alphanumeric() || byte == b'_'
}

fn strip_line_comments(line: &str) -> String {
  let mut in_block_comment = false;

  strip_line_comments_with_state(line, &mut in_block_comment)
}

fn strip_line_comments_with_state(line: &str, in_block_comment: &mut bool) -> String {
  let bytes = line.as_bytes();
  let mut sanitized = String::with_capacity(line.len());
  let mut cursor = 0;

  while cursor < bytes.len() {
    if *in_block_comment {
      if cursor + 1 < bytes.len() && bytes[cursor] == b'*' && bytes[cursor + 1] == b'/' {
        *in_block_comment = false;
        cursor += 2;
        continue;
      }

      cursor += 1;
      continue;
    }

    if cursor + 1 < bytes.len() && bytes[cursor] == b'/' && bytes[cursor + 1] == b'/' {
      break;
    }

    if cursor + 1 < bytes.len() && bytes[cursor] == b'/' && bytes[cursor + 1] == b'*' {
      *in_block_comment = true;
      cursor += 2;

      continue;
    }

    sanitized.push(char::from(bytes[cursor]));
    cursor += 1;
  }

  sanitized
}

fn strip_string_and_char_literals(line: &str) -> String {
  let bytes = line.as_bytes();
  let mut sanitized = String::with_capacity(line.len());
  let mut cursor = 0;

  while cursor < bytes.len() {
    let byte = bytes[cursor];

    if byte == b'"' || byte == b'\'' {
      let quote = byte;
      let mut escaped = false;

      cursor += 1;
      sanitized.push(' ');

      while cursor < bytes.len() {
        let current = bytes[cursor];

        if escaped {
          escaped = false;
          cursor += 1;
          continue;
        }

        if current == b'\\' {
          escaped = true;
          cursor += 1;
          continue;
        }

        cursor += 1;

        if current == quote {
          break;
        }
      }

      continue;
    }

    sanitized.push(char::from(byte));
    cursor += 1;
  }

  sanitized
}

fn consume_parenthesized_group(bytes: &[u8], cursor: &mut usize) -> bool {
  if *cursor >= bytes.len() || bytes[*cursor] != b'(' {
    return false;
  }

  let mut depth: usize = 0;

  while *cursor < bytes.len() {
    if bytes[*cursor] == b'(' {
      depth += 1;
    } else if bytes[*cursor] == b')' {
      if depth == 0 {
        return false;
      }

      depth -= 1;

      if depth == 0 {
        *cursor += 1;

        return true;
      }
    }

    *cursor += 1;
  }

  false
}

fn token_supports_parenthesized_suffix_args(token: &[u8]) -> bool {
  matches!(
    token,
    b"__attribute__" | b"__attribute" | b"__asm__" | b"__asm" | b"__declspec"
  )
}

fn skip_function_suffix_annotations(bytes: &[u8], cursor: &mut usize) {
  loop {
    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
      *cursor += 1;
    }

    if *cursor >= bytes.len() || bytes[*cursor] == b'(' {
      return;
    }

    if !(bytes[*cursor].is_ascii_alphabetic() || bytes[*cursor] == b'_') {
      return;
    }

    let token_start = *cursor;

    *cursor += 1;

    while *cursor < bytes.len() && is_identifier_byte(bytes[*cursor]) {
      *cursor += 1;
    }

    let token_end = *cursor;
    let token = &bytes[token_start..token_end];

    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
      *cursor += 1;
    }

    if !token_supports_parenthesized_suffix_args(token) {
      continue;
    }

    while *cursor < bytes.len() && bytes[*cursor] == b'(' {
      if !consume_parenthesized_group(bytes, cursor) {
        return;
      }
    }
  }
}

fn trailing_identifier(text: &str) -> Option<&str> {
  let mut end = text.len();
  let text_bytes = text.as_bytes();

  while end > 0 && !is_identifier_byte(text_bytes[end - 1]) {
    end -= 1;
  }

  if end == 0 {
    return None;
  }

  let mut start = end;

  while start > 0 && is_identifier_byte(text_bytes[start - 1]) {
    start -= 1;
  }

  Some(&text[start..end])
}

fn keyword_followed_by_parenthesized_expression(text: &str, keyword: &str) -> bool {
  let Some(rest) = text.strip_prefix(keyword) else {
    return false;
  };

  rest.trim_start().starts_with('(')
}

fn keyword_followed_by_expression(text: &str, keyword: &str) -> bool {
  let Some(rest) = text.strip_prefix(keyword) else {
    return false;
  };

  rest
    .chars()
    .next()
    .is_some_and(|character| character.is_ascii_whitespace() || character == '(')
}

fn prefix_is_control_statement_context(prefix: &str) -> bool {
  let trimmed = prefix.trim_start();

  if keyword_followed_by_expression(trimmed, "return")
    || keyword_followed_by_expression(trimmed, "case")
  {
    return true;
  }

  if keyword_followed_by_parenthesized_expression(trimmed, "if")
    || keyword_followed_by_parenthesized_expression(trimmed, "while")
    || keyword_followed_by_parenthesized_expression(trimmed, "for")
    || keyword_followed_by_parenthesized_expression(trimmed, "switch")
    || keyword_followed_by_parenthesized_expression(trimmed, "sizeof")
  {
    return true;
  }

  if let Some(after_else) = trimmed.strip_prefix("else") {
    return keyword_followed_by_parenthesized_expression(after_else.trim_start(), "if");
  }

  false
}

fn prefix_is_label_statement_context(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  trimmed.ends_with(':') && !trimmed.contains('?')
}

fn symbol_in_function_pointer_declarator(bytes: &[u8], symbol_start: usize) -> bool {
  let mut cursor = symbol_start;

  loop {
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
      cursor -= 1;
    }

    let mut consumed_identifier = false;

    while cursor > 0 && is_identifier_byte(bytes[cursor - 1]) {
      cursor -= 1;
      consumed_identifier = true;
    }

    if !consumed_identifier {
      break;
    }
  }

  while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
    cursor -= 1;
  }

  if cursor == 0 || bytes[cursor - 1] != b'*' {
    return false;
  }

  cursor -= 1;

  while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
    cursor -= 1;
  }

  cursor > 0 && bytes[cursor - 1] == b'('
}

fn symbol_in_member_access_expression(bytes: &[u8], symbol_start: usize) -> bool {
  let mut cursor = symbol_start;

  while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
    cursor -= 1;
  }

  if cursor == 0 {
    return false;
  }

  if bytes[cursor - 1] == b'.' {
    return true;
  }

  cursor >= 2 && bytes[cursor - 2] == b'-' && bytes[cursor - 1] == b'>'
}

fn prefix_has_ternary_question(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  if trimmed.ends_with('?') {
    return true;
  }

  strip_trailing_prefix_expression_wrappers(trimmed).ends_with('?')
}

fn strip_trailing_prefix_expression_wrappers(prefix: &str) -> &str {
  let mut trimmed = prefix.trim_end();

  while let Some(stripped) = trimmed.strip_suffix(|character: char| {
    matches!(
      character,
      '(' | '!' | '~' | '+' | '-' | '*' | '&' | '^' | '<' | '>' | '/'
    )
  }) {
    let next = stripped.trim_end();

    if next.len() == trimmed.len() {
      break;
    }

    trimmed = next;
  }

  trimmed
}

fn prefix_has_ternary_colon(prefix: &str) -> bool {
  let trimmed = strip_trailing_prefix_expression_wrappers(prefix);

  trimmed.ends_with(':') && trimmed.contains('?')
}

fn prefix_has_argument_separator(prefix: &str) -> bool {
  prefix.trim_end().ends_with(',')
}

fn strip_trailing_paren_and_unary_wrappers(prefix: &str) -> &str {
  let mut trimmed = prefix.trim_end();

  loop {
    let stripped = trimmed.trim_end_matches(['(', '!', '~', '+', '-', '*']);

    if stripped.len() == trimmed.len() {
      return trimmed;
    }

    trimmed = stripped.trim_end();
  }
}

fn prefix_has_logical_operator_suffix(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  if trimmed.ends_with("&&") || trimmed.ends_with("||") {
    return true;
  }

  let trimmed = strip_trailing_paren_and_unary_wrappers(trimmed);

  trimmed.ends_with("&&") || trimmed.ends_with("||")
}

fn prefix_has_unary_not_suffix(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  if trimmed.ends_with('!') || trimmed.ends_with('~') {
    return true;
  }

  let mut normalized = trimmed;

  while let Some(stripped) = normalized.strip_suffix('(') {
    normalized = stripped.trim_end();
  }

  normalized.ends_with('!') || normalized.ends_with('~')
}

fn prefix_has_bitwise_operator_suffix(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  if trimmed.ends_with("&&") || trimmed.ends_with("||") {
    return false;
  }

  if trimmed.ends_with('&') || trimmed.ends_with('|') || trimmed.ends_with('^') {
    return true;
  }

  let trimmed = strip_trailing_paren_and_unary_wrappers(trimmed);

  if trimmed.ends_with("&&") || trimmed.ends_with("||") {
    return false;
  }

  trimmed.ends_with('&') || trimmed.ends_with('|') || trimmed.ends_with('^')
}

fn prefix_has_arithmetic_or_relational_suffix(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();

  if trimmed.ends_with("->") || trimmed.ends_with("&&") || trimmed.ends_with("||") {
    return false;
  }

  if trimmed.ends_with('+')
    || trimmed.ends_with('-')
    || trimmed.ends_with('/')
    || trimmed.ends_with('%')
    || trimmed.ends_with('<')
    || trimmed.ends_with('>')
  {
    return true;
  }

  let mut normalized = trimmed;

  while let Some(stripped) = normalized.strip_suffix('(') {
    normalized = stripped.trim_end();
  }

  normalized.ends_with('+')
    || normalized.ends_with('-')
    || normalized.ends_with('/')
    || normalized.ends_with('%')
    || normalized.ends_with('<')
    || normalized.ends_with('>')
}

fn prefix_looks_like_cast_expression(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();
  let bytes = trimmed.as_bytes();

  if bytes.is_empty() || bytes[bytes.len() - 1] != b')' {
    return false;
  }

  let mut depth: usize = 0;
  let mut open_index: Option<usize> = None;

  for index in (0..bytes.len()).rev() {
    if bytes[index] == b')' {
      depth += 1;
      continue;
    }

    if bytes[index] != b'(' {
      continue;
    }

    if depth == 0 {
      return false;
    }

    depth -= 1;

    if depth == 0 {
      open_index = Some(index);
      break;
    }
  }

  let Some(open_index) = open_index else {
    return false;
  };

  if open_index == 0 {
    return true;
  }

  let mut before_open = open_index;

  while before_open > 0 && bytes[before_open - 1].is_ascii_whitespace() {
    before_open -= 1;
  }

  if before_open == 0 {
    return true;
  }

  let previous = bytes[before_open - 1];

  !(is_identifier_byte(previous) || previous == b')' || previous == b']')
}

fn prefix_looks_like_subscript_expression(prefix: &str) -> bool {
  prefix.trim_end().ends_with('[')
}

fn skip_trailing_whitespace(bytes: &[u8], cursor: &mut usize) {
  while *cursor > 0 && bytes[*cursor - 1].is_ascii_whitespace() {
    *cursor -= 1;
  }
}

fn skip_trailing_unary_operators(bytes: &[u8], cursor: &mut usize) {
  while *cursor > 0 && matches!(bytes[*cursor - 1], b'!' | b'~' | b'+' | b'-' | b'*' | b'&') {
    *cursor -= 1;
    skip_trailing_whitespace(bytes, cursor);
  }
}

fn consume_parenthesized_group_reverse(bytes: &[u8], cursor: &mut usize) -> bool {
  if *cursor == 0 || bytes[*cursor - 1] != b')' {
    return false;
  }

  let mut depth: usize = 0;
  let mut index = *cursor;

  while index > 0 {
    index -= 1;

    if bytes[index] == b')' {
      depth += 1;
      continue;
    }

    if bytes[index] != b'(' {
      continue;
    }

    if depth == 0 {
      return false;
    }

    depth -= 1;

    if depth == 0 {
      *cursor = index;

      return true;
    }
  }

  false
}

fn prefix_looks_like_call_expression(prefix: &str) -> bool {
  let trimmed = prefix.trim_end();
  let bytes = trimmed.as_bytes();

  if bytes.len() < 2 {
    return false;
  }

  let mut cursor = bytes.len();

  skip_trailing_whitespace(bytes, &mut cursor);
  skip_trailing_unary_operators(bytes, &mut cursor);

  while cursor > 0 && bytes[cursor - 1] == b')' {
    if !consume_parenthesized_group_reverse(bytes, &mut cursor) {
      return false;
    }

    skip_trailing_whitespace(bytes, &mut cursor);
    skip_trailing_unary_operators(bytes, &mut cursor);
  }

  if cursor == 0 || bytes[cursor - 1] != b'(' {
    return false;
  }

  let mut open_cursor = cursor;

  while open_cursor > 1 && bytes[open_cursor - 1] == b'(' {
    let previous = bytes[open_cursor - 2];

    if is_identifier_byte(previous) || previous == b')' || previous == b']' {
      return true;
    }

    open_cursor -= 1;
    skip_trailing_whitespace(bytes, &mut open_cursor);
  }

  if !bytes[..open_cursor].is_empty()
    && bytes[..open_cursor].iter().all(|byte| {
      byte.is_ascii_whitespace() || matches!(*byte, b'(' | b'!' | b'~' | b'+' | b'-' | b'*' | b'&')
    })
  {
    return true;
  }

  false
}

fn line_declares_function_symbol(line: &str, symbol: &str) -> bool {
  let bytes = line.as_bytes();
  let mut search_start = 0;

  while let Some(offset) = line[search_start..].find(symbol) {
    let start = search_start + offset;
    let end = start + symbol.len();
    let previous_is_identifier = start > 0 && is_identifier_byte(bytes[start - 1]);
    let next_is_identifier = end < bytes.len() && is_identifier_byte(bytes[end]);
    let has_assignment_before_symbol = bytes[..start].contains(&b'=');
    let prefix = line[..start].trim_end();
    let previous_keyword = trailing_identifier(prefix);
    let is_statement_context = matches!(
      previous_keyword,
      Some("return" | "if" | "while" | "for" | "switch" | "sizeof" | "case" | "do")
    );
    let is_control_statement_context = prefix_is_control_statement_context(prefix);
    let is_label_statement_context = prefix_is_label_statement_context(prefix);
    let in_function_pointer_declarator = symbol_in_function_pointer_declarator(bytes, start);
    let in_member_access_expression = symbol_in_member_access_expression(bytes, start);
    let in_ternary_expression = prefix_has_ternary_question(prefix);
    let in_ternary_colon_expression = prefix_has_ternary_colon(prefix);
    let in_argument_expression = prefix_has_argument_separator(prefix);
    let in_logical_expression = prefix_has_logical_operator_suffix(prefix);
    let in_unary_not_expression = prefix_has_unary_not_suffix(prefix);
    let in_bitwise_expression = prefix_has_bitwise_operator_suffix(prefix);
    let in_arithmetic_or_relational_expression = prefix_has_arithmetic_or_relational_suffix(prefix);
    let in_cast_expression = prefix_looks_like_cast_expression(prefix);
    let in_subscript_expression = prefix_looks_like_subscript_expression(prefix);
    let in_call_expression = prefix_looks_like_call_expression(prefix);

    if previous_is_identifier
      || next_is_identifier
      || has_assignment_before_symbol
      || is_statement_context
      || is_control_statement_context
      || is_label_statement_context
      || in_function_pointer_declarator
      || in_member_access_expression
      || in_ternary_expression
      || in_ternary_colon_expression
      || in_argument_expression
      || in_logical_expression
      || in_unary_not_expression
      || in_bitwise_expression
      || in_arithmetic_or_relational_expression
      || in_cast_expression
      || in_subscript_expression
      || in_call_expression
    {
      search_start = end;
      continue;
    }

    let mut cursor = end;

    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
      cursor += 1;
    }

    skip_function_suffix_annotations(bytes, &mut cursor);

    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
      cursor += 1;
    }

    while cursor < bytes.len() && bytes[cursor] == b')' {
      cursor += 1;

      while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
      }
    }

    if cursor < bytes.len() && bytes[cursor] == b'(' {
      return true;
    }

    search_start = end;
  }

  false
}

fn extern_token_declares_symbol(token: &str, symbol: &str) -> bool {
  let token = token
    .trim_matches('*')
    .trim_end_matches(';')
    .trim_end_matches(',');
  let token = token
    .split_once('[')
    .map_or(token, |(identifier, _)| identifier.trim_end());
  let token = token
    .split_once(')')
    .map_or(token, |(identifier, _)| identifier.trim_end());
  let token = token.trim_start_matches('(');
  let token = token.trim_start_matches('*');
  let token = token.trim_start_matches('&');
  let token = token
    .split_once('=')
    .map_or(token, |(identifier, _)| identifier.trim_end())
    .trim();

  token
    .split(|byte: char| !byte.is_ascii_alphanumeric() && byte != '_')
    .any(|part| !part.is_empty() && part == symbol)
}

fn line_declares_exported_symbol(line: &str, symbol: &str) -> bool {
  let without_comments = strip_line_comments(line);
  let without_literals = strip_string_and_char_literals(&without_comments);
  let trimmed = without_literals.trim();

  if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("typedef ") {
    return false;
  }

  if (trimmed.starts_with("struct ")
    || trimmed.starts_with("union ")
    || trimmed.starts_with("enum "))
    && (trimmed.ends_with('{') || trimmed.ends_with("};"))
  {
    return false;
  }

  if line_declares_function_symbol(trimmed, symbol) {
    return true;
  }

  if !trimmed.starts_with("extern ") || !trimmed.ends_with(';') || !trimmed.contains(symbol) {
    return false;
  }

  let declarator = trimmed
    .split_once('=')
    .map_or(trimmed, |(left, _)| left.trim_end());

  declarator
    .split_whitespace()
    .any(|token| extern_token_declares_symbol(token, symbol))
}

fn headers_declare_exported_symbol(all_headers: &str, symbol: &str) -> bool {
  let mut declaration = String::new();
  let mut in_block_comment = false;
  let mut in_macro_continuation = false;

  for line in all_headers.lines() {
    if in_macro_continuation {
      in_macro_continuation = line.trim_end().ends_with('\\');
      continue;
    }

    if line.trim().is_empty() {
      if line_declares_exported_symbol(&declaration, symbol) {
        return true;
      }

      declaration.clear();
      continue;
    }

    let without_comments = strip_line_comments_with_state(line, &mut in_block_comment);
    let without_literals = strip_string_and_char_literals(&without_comments);
    let trimmed = without_literals.trim();

    if trimmed.is_empty() {
      continue;
    }

    if trimmed.starts_with('#') {
      in_macro_continuation = trimmed.ends_with('\\');

      if line_declares_exported_symbol(&declaration, symbol) {
        return true;
      }

      declaration.clear();
      continue;
    }

    if !declaration.is_empty() {
      declaration.push(' ');
    }

    declaration.push_str(trimmed);

    if trimmed.ends_with(';') || trimmed.ends_with('{') || trimmed.ends_with("};") {
      if line_declares_exported_symbol(&declaration, symbol) {
        return true;
      }

      declaration.clear();
    }
  }

  line_declares_exported_symbol(&declaration, symbol)
}

#[test]
fn exported_symbol_detection_accepts_struct_returning_function_declarations() {
  assert!(line_declares_exported_symbol(
    "struct tm *gmtime(const time_t *timer);",
    "gmtime",
  ));
  assert!(line_declares_exported_symbol(
    "struct dirent *readdir(DIR *dirp);",
    "readdir",
  ));
}

#[test]
fn exported_symbol_detection_ignores_type_blocks_and_accepts_extern_variables() {
  assert!(!line_declares_exported_symbol(
    "struct sigaction {",
    "sigaction"
  ));
  assert!(!line_declares_exported_symbol(
    "enum thread_state {",
    "thread_state"
  ));
  assert!(!line_declares_exported_symbol(
    "typedef struct timespec timespec;",
    "timespec"
  ));
  assert!(line_declares_exported_symbol(
    "extern char **environ;",
    "environ"
  ));
}

#[test]
fn exported_symbol_detection_accepts_space_before_function_parenthesis() {
  assert!(line_declares_exported_symbol(
    "int getpid (void);",
    "getpid"
  ));
}

#[test]
fn exported_symbol_detection_rejects_substring_symbol_matches() {
  assert!(!line_declares_exported_symbol(
    "int pthread_cond_waiting(void);",
    "pthread_cond_wait",
  ));
}

#[test]
fn exported_symbol_detection_rejects_identifier_prefixed_matches() {
  assert!(!line_declares_exported_symbol(
    "int __real_getpid(void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_tab_before_function_parenthesis() {
  assert!(line_declares_exported_symbol(
    "int getppid\t(void);",
    "getppid",
  ));
}

#[test]
fn exported_symbol_detection_ignores_comment_only_lines() {
  assert!(!line_declares_exported_symbol(
    "/* int getpid(void); */",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol(
    "// int getppid(void);",
    "getppid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_declaration_with_inline_comments() {
  assert!(line_declares_exported_symbol(
    "int getpid/* libc attribute marker */(void);",
    "getpid",
  ));
  assert!(line_declares_exported_symbol(
    "int getppid(void); // declaration note",
    "getppid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_function_parenthesis_on_next_line() {
  let header = "int getpid\n(void);";

  assert!(headers_declare_exported_symbol(header, "getpid"));
}

#[test]
fn exported_symbol_detection_accepts_declaration_split_after_extern_type() {
  let header = "extern char **\nenviron;";

  assert!(headers_declare_exported_symbol(header, "environ"));
}

#[test]
fn exported_symbol_detection_accepts_function_split_across_three_lines() {
  let header = "int\ngetpid\n(void);";

  assert!(headers_declare_exported_symbol(header, "getpid"));
}

#[test]
fn exported_symbol_detection_accepts_function_split_with_comment_only_middle_line() {
  let header = "int getpid\n/* comment-only line */\n(void);";

  assert!(headers_declare_exported_symbol(header, "getpid"));
}

#[test]
fn exported_symbol_detection_accepts_function_with_throw_suffix_macro() {
  assert!(line_declares_exported_symbol(
    "int getpid __THROW (void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_function_with_attribute_suffix_macro() {
  assert!(line_declares_exported_symbol(
    "int getpid __attribute__((nonnull(1))) (void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_rejects_suffix_macro_for_different_symbol_token() {
  assert!(!line_declares_exported_symbol(
    "int getpid_alias __THROW (void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_function_with_multiple_suffix_macros() {
  assert!(line_declares_exported_symbol(
    "int getpid __THROW __wur (void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_function_with_mixed_suffix_macro_kinds() {
  assert!(line_declares_exported_symbol(
    "int getpid __attribute__((nonnull(1))) __THROW (void);",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_accepts_function_split_around_multiline_block_comment() {
  let header = "int getpid /* start\ncontinuation */ (void);";

  assert!(headers_declare_exported_symbol(header, "getpid"));
}

#[test]
fn exported_symbol_detection_ignores_symbol_text_inside_multiline_block_comment() {
  let header = "/* int getpid(void);\n   still comment */\nint getppid(void);";

  assert!(!headers_declare_exported_symbol(header, "getpid"));
  assert!(headers_declare_exported_symbol(header, "getppid"));
}

#[test]
fn exported_symbol_detection_accepts_extern_array_declaration() {
  assert!(line_declares_exported_symbol(
    "extern char *sys_errlist[];",
    "sys_errlist",
  ));
}

#[test]
fn exported_symbol_detection_rejects_extern_array_declaration_for_different_symbol() {
  assert!(!line_declares_exported_symbol(
    "extern char *sys_errlist_alias[];",
    "sys_errlist",
  ));
}

#[test]
fn exported_symbol_detection_accepts_extern_function_pointer_declaration() {
  assert!(line_declares_exported_symbol(
    "extern int (*signal_handler)(int);",
    "signal_handler",
  ));
}

#[test]
fn exported_symbol_detection_rejects_extern_function_pointer_for_different_symbol() {
  assert!(!line_declares_exported_symbol(
    "extern int (*signal_handler)(int);",
    "signal",
  ));
}

#[test]
fn exported_symbol_detection_rejects_function_call_text_in_extern_initializer() {
  assert!(!line_declares_exported_symbol(
    "extern int marker = getpid();",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_still_accepts_extern_function_declaration() {
  assert!(line_declares_exported_symbol(
    "extern int getpid(void);",
    "getpid",
  ));
}

#[test]
fn function_symbol_detection_accepts_parenthesized_function_declarator() {
  assert!(line_declares_function_symbol(
    "int (getpid)(void);",
    "getpid"
  ));
}

#[test]
fn function_symbol_detection_rejects_function_pointer_declarator() {
  assert!(!line_declares_function_symbol(
    "extern int (*signal_handler)(int);",
    "signal_handler",
  ));
  assert!(line_declares_exported_symbol(
    "extern int (*signal_handler)(int);",
    "signal_handler",
  ));
}

#[test]
fn function_symbol_detection_rejects_qualified_function_pointer_declarator() {
  assert!(!line_declares_function_symbol(
    "extern int (*const signal_handler)(int);",
    "signal_handler",
  ));
  assert!(line_declares_exported_symbol(
    "extern int (*const signal_handler)(int);",
    "signal_handler",
  ));
}

#[test]
fn exported_symbol_detection_ignores_function_like_text_in_macro_continuation() {
  let header = "#define WRAP_CALL \\\n  getpid(0) + \\\n  1\nextern int getppid(void);";

  assert!(!headers_declare_exported_symbol(header, "getpid"));
  assert!(headers_declare_exported_symbol(header, "getppid"));
}

#[test]
fn exported_symbol_detection_rejects_statement_context_function_call_text() {
  assert!(!line_declares_exported_symbol("return getpid();", "getpid"));
  assert!(!line_declares_exported_symbol(
    "return wrapper(getpid());",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol(
    "return\twrapper(getpid());",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol("if (getpid()) {", "getpid"));
  assert!(!line_declares_exported_symbol(
    "if (ready && getpid()) {",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol(
    "for (; getpid(); ) {",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "else if (getpid()) {",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol("do getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("case getpid():", "getpid"));
  assert!(!line_declares_exported_symbol("label: getpid();", "getpid"));
  assert!(!line_declares_exported_symbol(
    "case wrapper(getpid()):",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol("obj->getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("obj.getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("obj -> getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("obj . getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("(obj)->getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("(*obj).getpid();", "getpid"));
  assert!(!line_declares_exported_symbol(
    "ready ? getpid() : 0;",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready ? (getpid()) : 0;",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready ? 1 : getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready ? 1 : (getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready && getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready || (getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready & getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready + getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready + (getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready <= getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready == getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready != (getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready << getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "ready >> getpid();",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol("wrap(getpid());", "getpid"));
  assert!(!line_declares_exported_symbol(
    "wrap((getpid()));",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol("wrap(!getpid());", "getpid"));
  assert!(!line_declares_exported_symbol("!(getpid());", "getpid"));
  assert!(!line_declares_exported_symbol(
    "wrap(!(getpid()));",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol("!getpid();", "getpid"));
  assert!(!line_declares_exported_symbol("~getpid();", "getpid"));
  assert!(!line_declares_exported_symbol(
    "wrap(0,getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "wrap(0, getpid());",
    "getpid"
  ));
  assert!(!line_declares_exported_symbol(
    "wrap((int)getpid());",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol(
    "((fn_t)getpid)(0);",
    "getpid",
  ));
  assert!(!line_declares_exported_symbol("((getpid))(0);", "getpid",));
  assert!(!line_declares_exported_symbol("arr[getpid()];", "getpid"));
}

#[test]
fn exported_symbol_detection_rejects_function_like_text_inside_string_literal() {
  assert!(!line_declares_exported_symbol(
    "extern const char *marker = \"getpid(\";",
    "getpid",
  ));
}

#[test]
fn exported_symbol_detection_rejects_function_like_text_inside_char_literal() {
  assert!(!line_declares_exported_symbol(
    "extern int marker = 'g'; /* getpid( */",
    "getpid",
  ));
}

#[test]
fn required_header_files_exist() {
  for header_name in [
    "stddef.h",
    "stdarg.h",
    "errno.h",
    "dlfcn.h",
    "fenv.h",
    "math.h",
    "setjmp.h",
    "ctype.h",
    "locale.h",
    "netdb.h",
    "pthread.h",
    "signal.h",
    "stdio.h",
    "glob.h",
    "string.h",
    "stdlib.h",
    "time.h",
    "wchar.h",
    "fcntl.h",
    "dirent.h",
    "unistd.h",
    "sys/resource.h",
    "sys/socket.h",
    "sys/stat.h",
    "sys/utsname.h",
    "sys/sysinfo.h",
  ] {
    let path = include_root().join(header_name);

    assert!(
      path.is_file(),
      "missing header scaffold file: {}",
      path.display()
    );
  }
}

#[test]
fn signal_header_covers_delivery_and_mask_symbols() {
  let signal_header = read_header("signal.h");

  assert!(signal_header.contains("#ifndef RLIBC_SIGNAL_H"));
  assert!(signal_header.contains("typedef struct {"));
  assert!(signal_header.contains("unsigned long __val[16];"));
  assert!(signal_header.contains("} sigset_t;"));
  assert!(signal_header.contains("#define SIG_BLOCK 0"));
  assert!(signal_header.contains("#define SIG_UNBLOCK 1"));
  assert!(signal_header.contains("#define SIG_SETMASK 2"));
  assert!(signal_header.contains("#define SIGABRT 6"));
  assert!(signal_header.contains("#define SIGUSR1 10"));
  assert!(signal_header.contains("#define SA_SIGINFO 0x00000004UL"));
  assert!(signal_header.contains("#define SA_RESTORER 0x04000000UL"));
  assert!(signal_header.contains("#define SA_RESTART 0x10000000UL"));
  assert!(signal_header.contains("int sigemptyset(sigset_t *set);"));
  assert!(signal_header.contains("int sigfillset(sigset_t *set);"));
  assert!(signal_header.contains("int sigaddset(sigset_t *set, int signum);"));
  assert!(signal_header.contains("int sigdelset(sigset_t *set, int signum);"));
  assert!(signal_header.contains("int sigismember(const sigset_t *set, int signum);"));
  assert!(
    signal_header.contains(
      "int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);"
    )
  );
  assert!(signal_header.contains("int raise(int sig);"));
  assert!(signal_header.contains("int kill(int pid, int sig);"));
  assert!(
    signal_header.contains("int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);")
  );
}

#[test]
fn netdb_header_covers_getaddrinfo_and_getnameinfo_contract() {
  let netdb_header = read_header("netdb.h");

  assert!(netdb_header.contains("#ifndef RLIBC_NETDB_H"));
  assert!(netdb_header.contains("#include <sys/socket.h>"));
  assert!(netdb_header.contains("#define AF_UNSPEC 0"));
  assert!(netdb_header.contains("#define AF_INET 2"));
  assert!(netdb_header.contains("#define AF_INET6 10"));
  assert!(netdb_header.contains("#define AI_PASSIVE 0x0001"));
  assert!(netdb_header.contains("#define AI_NUMERICHOST 0x0004"));
  assert!(netdb_header.contains("#define AI_NUMERICSERV 0x0400"));
  assert!(netdb_header.contains("#define NI_NUMERICHOST 0x01"));
  assert!(netdb_header.contains("#define NI_NUMERICSERV 0x02"));
  assert!(netdb_header.contains("#define NI_NAMEREQD 0x08"));
  assert!(netdb_header.contains("#define EAI_BADFLAGS (-1)"));
  assert!(netdb_header.contains("#define EAI_FAMILY (-6)"));
  assert!(netdb_header.contains("#define EAI_OVERFLOW (-12)"));
  assert!(netdb_header.contains("struct addrinfo {"));
  assert!(netdb_header.contains("int getaddrinfo("));
  assert!(netdb_header.contains("void freeaddrinfo(struct addrinfo *res);"));
  assert!(netdb_header.contains("const char *gai_strerror(int errcode);"));
  assert!(netdb_header.contains("int getnameinfo("));
}

#[test]
fn fenv_header_covers_base_fe_symbols() {
  let fenv_header = read_header("fenv.h");

  assert!(fenv_header.contains("#ifndef RLIBC_FENV_H"));
  assert!(fenv_header.contains("typedef unsigned short fexcept_t;"));
  assert!(fenv_header.contains("typedef struct {"));
  assert!(fenv_header.contains("unsigned int __opaque_words[8];"));
  assert!(fenv_header.contains("} fenv_t;"));
  assert!(fenv_header.contains("#define FE_ALL_EXCEPT 0x3d"));
  assert!(fenv_header.contains("#define FE_TONEAREST 0x0000"));
  assert!(fenv_header.contains("#define FE_DFL_ENV ((const fenv_t *)-1)"));
  assert!(fenv_header.contains("int feclearexcept(int excepts);"));
  assert!(fenv_header.contains("int feupdateenv(const fenv_t *envp);"));
}

#[test]
fn math_header_covers_i049_symbols() {
  let math_header = read_header("math.h");

  assert!(math_header.contains("#ifndef RLIBC_MATH_H"));
  assert!(math_header.contains("double sqrt(double x);"));
  assert!(math_header.contains("double log(double x);"));
  assert!(math_header.contains("double exp(double x);"));
}

#[test]
fn dirent_header_covers_directory_stream_symbols() {
  let dirent_header = read_header("dirent.h");

  assert!(dirent_header.contains("#ifndef RLIBC_DIRENT_H"));
  assert!(dirent_header.contains("typedef struct rlibc_dir DIR;"));
  assert!(dirent_header.contains("struct dirent {"));
  assert!(dirent_header.contains("char d_name[256];"));
  assert!(dirent_header.contains("DIR *opendir(const char *path);"));
  assert!(dirent_header.contains("struct dirent *readdir(DIR *dirp);"));
  assert!(dirent_header.contains("int closedir(DIR *dirp);"));
  assert!(dirent_header.contains("void rewinddir(DIR *dirp);"));
}

#[test]
fn setjmp_header_declares_jmp_buf_and_jump_functions() {
  let setjmp_header = read_header("setjmp.h");

  assert!(setjmp_header.contains("#ifndef RLIBC_SETJMP_H"));
  assert!(setjmp_header.contains("typedef long jmp_buf[8];"));
  assert!(setjmp_header.contains("int setjmp(jmp_buf env);"));
  assert!(
    setjmp_header.contains("RLIBC_NORETURN"),
    "setjmp.h should mark longjmp as noreturn for C callers",
  );
  assert!(setjmp_header.contains("RLIBC_NORETURN void longjmp(jmp_buf env, int value);"));
  assert!(
    setjmp_header.contains("defined(_MSC_VER)"),
    "setjmp.h should define an MSVC noreturn fallback branch",
  );
  assert!(
    setjmp_header.contains("__declspec(noreturn)"),
    "setjmp.h should map RLIBC_NORETURN to __declspec(noreturn) for MSVC C mode",
  );
  assert!(
    setjmp_header.contains("#ifdef RLIBC_NORETURN"),
    "setjmp.h should clear a pre-defined RLIBC_NORETURN helper before declaring its local fallback",
  );
  assert!(
    setjmp_header.contains("#undef RLIBC_NORETURN"),
    "setjmp.h should not leak helper noreturn macro outside header scope",
  );
}

#[test]
fn setjmp_header_clears_helper_before_include_guard() {
  let setjmp_header = read_header("setjmp.h");
  let clear_helper_pos = setjmp_header
    .find("#ifdef RLIBC_NORETURN")
    .unwrap_or_else(|| panic!("setjmp.h should start with helper macro cleanup guard"));
  let include_guard_pos = setjmp_header
    .find("#ifndef RLIBC_SETJMP_H")
    .unwrap_or_else(|| panic!("setjmp.h should declare include guard"));

  assert!(
    clear_helper_pos < include_guard_pos,
    "setjmp.h should clear pre-defined RLIBC_NORETURN before include guard short-circuits re-includes",
  );
}

#[test]
fn setjmp_header_does_not_leak_noreturn_helper_macro() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_scope_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN leaked from <setjmp.h>\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_reinclude_does_not_reintroduce_noreturn_helper_macro() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_reinclude_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#include <setjmp.h>",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN leaked after re-including <setjmp.h>\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_tolerates_predefined_noreturn_helper_macro() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_predefined_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#define RLIBC_NORETURN __attribute__((deprecated))",
    "#include <setjmp.h>",
    "",
    "int main(void) {",
    "  jmp_buf env = {0};",
    "  (void)setjmp(env);",
    "  if (0) {",
    "    longjmp(env, 1);",
    "  }",
    "  return 0;",
    "}",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_undefines_predefined_noreturn_helper_after_include() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_predefined_scope_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#define RLIBC_NORETURN __attribute__((deprecated))",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN should be undefined after including <setjmp.h>\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_reinclude_after_predefined_helper_keeps_macro_undefined() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_predefined_reinclude_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#define RLIBC_NORETURN __attribute__((deprecated))",
    "#include <setjmp.h>",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN should remain undefined after re-including <setjmp.h>\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_tolerates_function_like_predefined_noreturn_helper_macro() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_predefined_function_like_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#define RLIBC_NORETURN(...) __attribute__((deprecated))",
    "#include <setjmp.h>",
    "",
    "int main(void) {",
    "  jmp_buf env = {0};",
    "  (void)setjmp(env);",
    "  if (0) {",
    "    longjmp(env, 1);",
    "  }",
    "  return 0;",
    "}",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_undefines_function_like_predefined_noreturn_helper_after_include() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_predefined_function_like_scope_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#define RLIBC_NORETURN(...) __attribute__((deprecated))",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN should be undefined after including <setjmp.h>\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_reinclude_clears_helper_macro_redefined_after_first_include() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_redefined_between_includes_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#include <setjmp.h>",
    "#define RLIBC_NORETURN __attribute__((deprecated))",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN should be cleared on every <setjmp.h> include\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn setjmp_header_reinclude_clears_function_like_helper_macro_redefined_after_first_include() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_setjmp_noreturn_function_like_redefined_between_includes_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = [
    "#include <setjmp.h>",
    "#define RLIBC_NORETURN(...) __attribute__((deprecated))",
    "#include <setjmp.h>",
    "#ifdef RLIBC_NORETURN",
    "#error \"RLIBC_NORETURN should be cleared on every <setjmp.h> include\"",
    "#endif",
    "",
    "int main(void) { return 0; }",
    "",
  ]
  .join("\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn stddef_header_declares_size_types_and_null() {
  let header = read_header("stddef.h");

  assert!(header.contains("#ifndef RLIBC_STDDEF_H"));
  assert!(header.contains("typedef __SIZE_TYPE__ size_t;"));
  assert!(header.contains("typedef __PTRDIFF_TYPE__ ptrdiff_t;"));
  assert!(header.contains("#define NULL ((void *)0)"));
}

#[test]
fn errno_header_exposes_errno_location_contract() {
  let header = read_header("errno.h");

  assert!(header.contains("#ifndef RLIBC_ERRNO_H"));
  assert!(header.contains("int *__errno_location(void);"));
  assert!(header.contains("#define errno (*__errno_location())"));
}

#[test]
fn dlfcn_header_covers_minimal_dynamic_loader_symbols() {
  let dlfcn_header = read_header("dlfcn.h");

  assert!(dlfcn_header.contains("#ifndef RLIBC_DLFCN_H"));
  assert!(dlfcn_header.contains("#define RTLD_LAZY 0x0001"));
  assert!(dlfcn_header.contains("#define RTLD_NOW 0x0002"));
  assert!(dlfcn_header.contains("#define RTLD_GLOBAL 0x0100"));
  assert!(dlfcn_header.contains("#define RTLD_LOCAL 0"));
  assert!(dlfcn_header.contains("void *dlopen(const char *filename, int flags);"));
  assert!(dlfcn_header.contains("char *dlerror(void);"));
  assert!(dlfcn_header.contains("void *dlsym(void *handle, const char *symbol);"));
}

#[test]
fn ctype_and_locale_headers_cover_core_symbols() {
  let ctype_header = read_header("ctype.h");

  assert!(ctype_header.contains("#ifndef RLIBC_CTYPE_H"));
  assert!(ctype_header.contains("int isalpha(int c);"));
  assert!(ctype_header.contains("int isspace(int c);"));
  assert!(ctype_header.contains("int tolower(int c);"));
  assert!(ctype_header.contains("int toupper(int c);"));

  let locale_header = read_header("locale.h");

  assert!(locale_header.contains("#ifndef RLIBC_LOCALE_H"));
  assert!(locale_header.contains("#define LC_CTYPE 0"));
  assert!(locale_header.contains("#define LC_NUMERIC 1"));
  assert!(locale_header.contains("#define LC_TIME 2"));
  assert!(locale_header.contains("#define LC_COLLATE 3"));
  assert!(locale_header.contains("#define LC_MONETARY 4"));
  assert!(locale_header.contains("#define LC_MESSAGES 5"));
  assert!(locale_header.contains("#define LC_ALL 6"));
  assert!(locale_header.contains("char *setlocale(int category, const char *locale);"));
}

#[test]
fn stdio_header_covers_setvbuf_and_vsnprintf_symbols() {
  let stdio_header = read_header("stdio.h");

  assert!(stdio_header.contains("#ifndef RLIBC_STDIO_H"));
  assert!(stdio_header.contains("#define EOF (-1)"));
  assert!(stdio_header.contains("#define _IOFBF 0"));
  assert!(stdio_header.contains("#define _IOLBF 1"));
  assert!(stdio_header.contains("#define _IONBF 2"));
  assert!(stdio_header.contains("typedef struct FILE FILE;"));
  assert!(stdio_header.contains("int setvbuf(FILE *stream, char *buffer, int mode, size_t size);"));
  assert!(
    stdio_header.contains("int vsnprintf(char *s, size_t n, const char *format, va_list ap);")
  );
}

#[test]
fn string_and_stdlib_headers_cover_current_exported_symbols() {
  let string_header = read_header("string.h");

  assert!(string_header.contains("void *memmove(void *dst, const void *src, size_t n);"));
  assert!(string_header.contains("size_t strlen(const char *s);"));

  let stdlib_header = read_header("stdlib.h");

  assert!(stdlib_header.contains("extern char **environ;"));
  assert!(stdlib_header.contains("long strtol(const char *nptr, char **endptr, int base);"));
  assert!(stdlib_header.contains("int mblen(const char *s, size_t n);"));
  assert!(stdlib_header.contains("int mbtowc(wchar_t *pwc, const char *s, size_t n);"));
  assert!(stdlib_header.contains("int wctomb(char *s, wchar_t wc);"));
  assert!(stdlib_header.contains("size_t mbstowcs(wchar_t *dst, const char *src, size_t len);"));
  assert!(stdlib_header.contains("size_t wcstombs(char *dst, const wchar_t *src, size_t len);"));
  assert!(
    stdlib_header.contains("int setenv(const char *name, const char *value, int overwrite);")
  );
  assert!(stdlib_header.contains("void __libc_start_main(int (*main)(int, char **, char **), int argc, char **argv, char **envp);"));
  assert!(stdlib_header.contains("void _Exit(int status);"));
}

#[test]
fn glob_header_covers_pattern_and_release_symbols() {
  let glob_header = read_header("glob.h");

  assert!(glob_header.contains("#ifndef RLIBC_GLOB_H"));
  assert!(glob_header.contains("#define GLOB_ERR 0x0001"));
  assert!(glob_header.contains("#define GLOB_NOMATCH 3"));
  assert!(glob_header.contains("typedef struct {"));
  assert!(glob_header.contains("size_t gl_pathc;"));
  assert!(glob_header.contains("char **gl_pathv;"));
  assert!(glob_header.contains("int glob("));
  assert!(glob_header.contains("glob_t *pglob"));
  assert!(glob_header.contains("void globfree(glob_t *pglob);"));
}

#[test]
fn time_header_covers_time_and_calendar_symbols() {
  let time_header = read_header("time.h");

  assert!(time_header.contains("#ifndef RLIBC_TIME_H"));
  assert!(time_header.contains("typedef int clockid_t;"));
  assert!(time_header.contains("typedef long time_t;"));
  assert!(time_header.contains("#define CLOCK_REALTIME 0"));
  assert!(time_header.contains("#define CLOCK_MONOTONIC 1"));
  assert!(time_header.contains("#define CLOCK_PROCESS_CPUTIME_ID 2"));
  assert!(time_header.contains("#define CLOCK_THREAD_CPUTIME_ID 3"));
  assert!(time_header.contains("#define CLOCK_MONOTONIC_RAW 4"));
  assert!(time_header.contains("#define CLOCK_REALTIME_COARSE 5"));
  assert!(time_header.contains("#define CLOCK_MONOTONIC_COARSE 6"));
  assert!(time_header.contains("#define CLOCK_BOOTTIME 7"));
  assert!(time_header.contains("#define CLOCK_REALTIME_ALARM 8"));
  assert!(time_header.contains("#define CLOCK_BOOTTIME_ALARM 9"));
  assert!(time_header.contains("#define CLOCK_SGI_CYCLE 10"));
  assert!(time_header.contains("#define CLOCK_TAI 11"));
  assert!(time_header.contains("#define CLOCKFD 3"));
  assert!(time_header.contains("#define FD_TO_CLOCKID(fd)"));
  assert!(time_header.contains("#define CLOCKID_TO_FD(clk)"));
  assert!(time_header.contains("struct tm {"));
  assert!(time_header.contains("int clock_gettime(clockid_t clock_id, struct timespec *tp);"));
  assert!(time_header.contains("int gettimeofday(struct timeval *tv, struct timezone *tz);"));
  assert!(time_header.contains("struct tm *gmtime_r(const time_t *timer, struct tm *result);"));
  assert!(time_header.contains("struct tm *gmtime(const time_t *timer);"));
  assert!(time_header.contains("struct tm *localtime_r(const time_t *timer, struct tm *result);"));
  assert!(time_header.contains("struct tm *localtime(const time_t *timer);"));
  assert!(time_header.contains("time_t timegm(struct tm *time_parts);"));
  assert!(time_header.contains("time_t mktime(struct tm *time_parts);"));
  assert!(time_header.contains(
    "size_t strftime(char *s, size_t max, const char *format, const struct tm *time_ptr);"
  ));
}

#[test]
fn fcntl_unistd_and_system_headers_cover_file_and_system_symbols() {
  let fcntl_header = read_header("fcntl.h");

  assert!(fcntl_header.contains("#ifndef RLIBC_FCNTL_H"));
  assert!(fcntl_header.contains("#define F_DUPFD 0"));
  assert!(fcntl_header.contains("#define F_GETFD 1"));
  assert!(fcntl_header.contains("#define F_SETFD 2"));
  assert!(fcntl_header.contains("#define F_DUPFD_CLOEXEC 1030"));
  assert!(fcntl_header.contains("#define F_GETFL 3"));
  assert!(fcntl_header.contains("#define F_SETFL 4"));
  assert!(fcntl_header.contains("#define FD_CLOEXEC 1"));
  assert!(fcntl_header.contains("#define O_ACCMODE 03"));
  assert!(fcntl_header.contains("#define O_NONBLOCK 04000"));
  assert!(fcntl_header.contains("#define O_RDONLY 00"));
  assert!(fcntl_header.contains("#define AT_FDCWD -100"));
  assert!(fcntl_header.contains("#define AT_EMPTY_PATH 0x1000"));
  assert!(fcntl_header.contains("int fcntl(int fd, int cmd, ...);"));
  assert!(fcntl_header.contains("int open(const char *pathname, int flags, ...);"));
  assert!(fcntl_header.contains("int openat(int dirfd, const char *pathname, int flags, ...);"));

  let unistd_header = read_header("unistd.h");

  assert!(unistd_header.contains("#ifndef RLIBC_UNISTD_H"));
  assert!(unistd_header.contains("typedef __PTRDIFF_TYPE__ ssize_t;"));
  assert!(unistd_header.contains("#define MSG_PEEK 0x2"));
  assert!(unistd_header.contains("#define MSG_DONTWAIT 0x40"));
  assert!(unistd_header.contains("#define MSG_WAITALL 0x100"));
  assert!(unistd_header.contains("#define MSG_NOSIGNAL 0x4000"));
  assert!(unistd_header.contains("#define _SC_CLK_TCK 2"));
  assert!(unistd_header.contains("#define _SC_OPEN_MAX 4"));
  assert!(unistd_header.contains("#define _SC_PAGESIZE 30"));
  assert!(unistd_header.contains("#define _SC_NPROCESSORS_CONF 83"));
  assert!(unistd_header.contains("#define _SC_NPROCESSORS_ONLN 84"));
  assert!(unistd_header.contains("#define HOST_NAME_MAX 64"));
  assert!(unistd_header.contains("ssize_t read(int fd, void *buf, size_t count);"));
  assert!(unistd_header.contains("ssize_t write(int fd, const void *buf, size_t count);"));
  assert!(
    unistd_header.contains("ssize_t send(int sockfd, const void *buf, size_t len, int flags);")
  );
  assert!(unistd_header.contains("ssize_t recv(int sockfd, void *buf, size_t len, int flags);"));
  assert!(unistd_header.contains("int gethostname(char *name, size_t len);"));
  assert!(unistd_header.contains("int getpagesize(void);"));
  assert!(unistd_header.contains("long sysconf(int name);"));

  let stat_header = read_header("sys/stat.h");

  assert!(stat_header.contains("#ifndef RLIBC_SYS_STAT_H"));
  assert!(stat_header.contains("struct stat {"));
  assert!(stat_header.contains("int stat(const char *path, struct stat *stat_buf);"));
  assert!(stat_header.contains("int fstat(int fd, struct stat *stat_buf);"));
  assert!(stat_header.contains("int lstat(const char *path, struct stat *stat_buf);"));
  assert!(
    stat_header.contains("int fstatat(int fd, const char *path, struct stat *stat_buf, int flag);")
  );
  assert!(stat_header.contains("#define AT_SYMLINK_NOFOLLOW 0x100"));
  assert!(stat_header.contains("#define AT_EMPTY_PATH 0x1000"));

  let socket_header = read_header("sys/socket.h");

  assert!(socket_header.contains("#ifndef RLIBC_SYS_SOCKET_H"));
  assert!(socket_header.contains("typedef unsigned short sa_family_t;"));
  assert!(socket_header.contains("typedef unsigned int socklen_t;"));
  assert!(socket_header.contains("struct sockaddr {"));
  assert!(socket_header.contains("struct sockaddr_un {"));
  assert!(socket_header.contains("#define AF_UNIX 1"));
  assert!(socket_header.contains("#define SOCK_STREAM 1"));
  assert!(socket_header.contains("#define SOCK_NONBLOCK 04000"));
  assert!(socket_header.contains("#define SOCK_CLOEXEC 02000000"));
  assert!(socket_header.contains("int socket(int domain, int type, int protocol);"));
  assert!(
    socket_header
      .contains("int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen);")
  );
  assert!(
    socket_header.contains("int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen);")
  );
  assert!(socket_header.contains("int listen(int sockfd, int backlog);"));
  assert!(
    socket_header.contains("int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen);")
  );

  let utsname_header = read_header("sys/utsname.h");

  assert!(utsname_header.contains("#ifndef RLIBC_SYS_UTSNAME_H"));
  assert!(utsname_header.contains("#define __OLD_UTS_LEN 8"));
  assert!(utsname_header.contains("#define __NEW_UTS_LEN 64"));
  assert!(utsname_header.contains("#define _UTSNAME_LENGTH (__NEW_UTS_LEN + 1)"));
  assert!(utsname_header.contains("#define _UTSNAME_SYSNAME_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define _UTSNAME_NODENAME_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define _UTSNAME_RELEASE_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define _UTSNAME_VERSION_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define _UTSNAME_MACHINE_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define _UTSNAME_DOMAIN_LENGTH _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("#define SYS_NMLN _UTSNAME_LENGTH"));
  assert!(utsname_header.contains("struct utsname {"));
  assert!(utsname_header.contains("char sysname[_UTSNAME_SYSNAME_LENGTH];"));
  assert!(utsname_header.contains("char nodename[_UTSNAME_NODENAME_LENGTH];"));
  assert!(utsname_header.contains("char release[_UTSNAME_RELEASE_LENGTH];"));
  assert!(utsname_header.contains("char version[_UTSNAME_VERSION_LENGTH];"));
  assert!(utsname_header.contains("char machine[_UTSNAME_MACHINE_LENGTH];"));
  assert!(utsname_header.contains("char domainname[_UTSNAME_DOMAIN_LENGTH];"));
  assert!(utsname_header.contains("int uname(struct utsname *buf);"));

  let sysinfo_header = read_header("sys/sysinfo.h");

  assert!(sysinfo_header.contains("#ifndef RLIBC_SYS_SYSINFO_H"));
  assert!(sysinfo_header.contains("#define SI_LOAD_SHIFT 16"));
  assert!(sysinfo_header.contains("struct sysinfo {"));
  assert!(sysinfo_header.contains("int sysinfo(struct sysinfo *info);"));

  let resource_header = read_header("sys/resource.h");

  assert!(resource_header.contains("#ifndef RLIBC_SYS_RESOURCE_H"));
  assert!(resource_header.contains("struct rlimit {"));
  assert!(resource_header.contains("int getrlimit(int resource, struct rlimit *rlim);"));
  assert!(resource_header.contains("int setrlimit(int resource, const struct rlimit *rlim);"));
  assert!(
    resource_header
      .contains("int prlimit64(int pid, int resource, const struct rlimit *new_limit, struct rlimit *old_limit);")
  );
}

#[test]
fn public_headers_compile_as_c_translation_unit() {
  let compiler = find_c_compiler()
    .unwrap_or_else(|| panic!("no C compiler found in PATH (checked CC, cc, clang, gcc)"));
  let source = all_header_paths_relative()
    .into_iter()
    .map(|header| format!("#include <{header}>"))
    .collect::<Vec<_>>()
    .join("\n");
  let nonce = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let source_path = std::env::temp_dir().join(format!(
    "rlibc_public_headers_smoke_{}_{}.c",
    std::process::id(),
    nonce
  ));
  let translation_unit = format!("{source}\n\nint main(void) {{ return 0; }}\n");

  std::fs::write(&source_path, translation_unit)
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", source_path.display()));

  let output = Command::new(&compiler)
    .arg("-std=c11")
    .arg("-fsyntax-only")
    .arg("-I")
    .arg(include_root())
    .arg(&source_path)
    .output()
    .unwrap_or_else(|error| panic!("failed to execute {compiler}: {error}"));
  let _ = std::fs::remove_file(&source_path);

  assert!(
    output.status.success(),
    "{compiler} failed for {}.\nstdout:\n{}\nstderr:\n{}",
    source_path.display(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn dlfcn_header_covers_dlsym_symbol() {
  let dlfcn_header = read_header("dlfcn.h");

  assert!(dlfcn_header.contains("#ifndef RLIBC_DLFCN_H"));
  assert!(dlfcn_header.contains("char *dlerror(void);"));
  assert!(dlfcn_header.contains("void *dlsym(void *handle, const char *symbol);"));
}

#[test]
fn pthread_header_covers_rwlock_symbols() {
  let pthread_header = read_header("pthread.h");

  assert!(pthread_header.contains("#ifndef RLIBC_PTHREAD_H"));
  assert!(pthread_header.contains("typedef unsigned long pthread_t;"));
  assert!(pthread_header.contains("pthread_attr_t;"));
  assert!(pthread_header.contains("int pthread_create("));
  assert!(pthread_header.contains("int pthread_join(pthread_t thread, void **retval);"));
  assert!(pthread_header.contains("int pthread_detach(pthread_t thread);"));
  assert!(pthread_header.contains("typedef union"));
  assert!(pthread_header.contains("pthread_rwlock_t;"));
  assert!(pthread_header.contains("pthread_rwlockattr_t;"));
  assert!(pthread_header.contains(
    "int pthread_rwlock_init(pthread_rwlock_t *rwlock, const pthread_rwlockattr_t *attr);"
  ));
  assert!(pthread_header.contains("int pthread_rwlock_destroy(pthread_rwlock_t *rwlock);"));
  assert!(pthread_header.contains("int pthread_rwlock_rdlock(pthread_rwlock_t *rwlock);"));
  assert!(pthread_header.contains("int pthread_rwlock_tryrdlock(pthread_rwlock_t *rwlock);"));
  assert!(pthread_header.contains("int pthread_rwlock_wrlock(pthread_rwlock_t *rwlock);"));
  assert!(pthread_header.contains("int pthread_rwlock_trywrlock(pthread_rwlock_t *rwlock);"));
  assert!(pthread_header.contains("int pthread_rwlock_unlock(pthread_rwlock_t *rwlock);"));
}

#[test]
fn wchar_header_covers_restartable_multibyte_symbols() {
  let wchar_header = read_header("wchar.h");

  assert!(wchar_header.contains("#ifndef RLIBC_WCHAR_H"));
  assert!(wchar_header.contains("typedef int wchar_t;"));
  assert!(wchar_header.contains("} mbstate_t;"));
  assert!(
    wchar_header.contains("size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);")
  );
  assert!(wchar_header.contains("size_t mbrlen(const char *s, size_t n, mbstate_t *ps);"));
  assert!(wchar_header.contains("int mbsinit(const mbstate_t *ps);"));
  assert!(wchar_header.contains("int mblen(const char *s, size_t n);"));
  assert!(wchar_header.contains("int mbtowc(wchar_t *pwc, const char *s, size_t n);"));
  assert!(wchar_header.contains("int wctomb(char *s, wchar_t wc);"));
  assert!(wchar_header.contains("size_t mbstowcs(wchar_t *dst, const char *src, size_t len);"));
  assert!(wchar_header.contains("size_t wcstombs(char *dst, const wchar_t *src, size_t len);"));
}

#[test]
fn public_headers_declare_all_exported_symbols_from_map() {
  let all_headers = all_header_text();
  let missing_symbols = exported_symbols_from_map()
    .into_iter()
    .filter(|symbol| !headers_declare_exported_symbol(&all_headers, symbol))
    .collect::<Vec<_>>();

  assert!(
    missing_symbols.is_empty(),
    "missing public header declarations for exported symbols: {missing_symbols:?}",
  );
}
