#[derive(Clone, Debug)]
pub(super) struct TextEdit {
  pub(super) start_byte: usize,
  pub(super) end_byte: usize,
  pub(super) replacement: String,
}

pub(super) fn apply_edits(source: &str, edits: &mut [TextEdit]) -> Option<String> {
  if edits.is_empty() {
    return None;
  }

  edits.sort_by_key(|edit| edit.start_byte);

  let mut updated = source.to_owned();

  for edit in edits.iter().rev() {
    updated.replace_range(edit.start_byte..edit.end_byte, &edit.replacement);
  }

  if updated == source {
    None
  } else {
    Some(updated)
  }
}

pub(super) fn detect_newline_sequence(source: &str) -> &'static str {
  source.find('\n').map_or("\n", |index| {
    if index > 0 && source.as_bytes()[index - 1] == b'\r' {
      "\r\n"
    } else {
      "\n"
    }
  })
}

pub(super) fn line_start_offset(line_starts: &[usize], line_number: usize) -> usize {
  line_starts
    .get(line_number.saturating_sub(1))
    .copied()
    .unwrap_or_else(|| *line_starts.last().unwrap_or(&0))
}

pub(super) fn build_line_starts(source: &str) -> Vec<usize> {
  let mut line_starts = vec![0];

  for (index, byte) in source.bytes().enumerate() {
    if byte == b'\n' {
      line_starts.push(index + 1);
    }
  }

  line_starts.push(source.len());

  line_starts
}

pub(super) fn has_only_whitespace(source: &str, start_byte: usize, end_byte: usize) -> bool {
  source
    .get(start_byte..end_byte)
    .is_some_and(|text| text.chars().all(char::is_whitespace))
}
