use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleViolation {
  pub line: usize,
  pub column: usize,
  pub code: &'static str,
  pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisResult {
  pub violations: Vec<StyleViolation>,
  pub formatted_source: Option<String>,
}

impl fmt::Display for StyleViolation {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(formatter, "{} {}", self.code, self.message)
  }
}
