use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::{env, fs};

const LEDGER_HEADER: [&str; 5] = ["suite", "case", "target", "reason", "expires"];
const RESULT_HEADER: [&str; 4] = ["suite", "case", "target", "status"];
const DEFAULT_LEDGER_PATH: &str = "docs/conformance/xfail-ledger.csv";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CaseKey {
  suite: String,
  case_id: String,
  target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawStatus {
  Pass,
  Fail,
  Skip,
  Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalStatus {
  Pass,
  Fail,
  Skip,
  Timeout,
  XFail,
  XPass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LedgerEntry {
  key: CaseKey,
  reason: String,
  expires: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawResult {
  key: CaseKey,
  status: RawStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassifiedResult {
  key: CaseKey,
  status: FinalStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CsvRecord {
  fields: Vec<String>,
  line_no: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Summary {
  pass: usize,
  fail: usize,
  skip: usize,
  timeout: usize,
  xfail: usize,
  xpass: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
  ledger_path: PathBuf,
  results_path: PathBuf,
  strict_xpass: bool,
  as_of_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
  Run(Config),
  Help,
}

impl Summary {
  const fn empty() -> Self {
    Self {
      pass: 0,
      fail: 0,
      skip: 0,
      timeout: 0,
      xfail: 0,
      xpass: 0,
    }
  }
}

fn main() -> ExitCode {
  let args: Vec<String> = env::args().skip(1).collect();

  match run(&args) {
    Ok(Some(summary)) => {
      println!(
        "summary pass={} fail={} skip={} timeout={} xfail={} xpass={}",
        summary.pass, summary.fail, summary.skip, summary.timeout, summary.xfail, summary.xpass,
      );
      ExitCode::SUCCESS
    }
    Ok(None) => ExitCode::SUCCESS,
    Err(message) => {
      eprintln!("{message}");
      ExitCode::FAILURE
    }
  }
}

fn run(args: &[String]) -> Result<Option<Summary>, String> {
  let action = parse_args(args)?;
  let Action::Run(config) = action else {
    print_usage();

    return Ok(None);
  };
  let ledger_text = fs::read_to_string(&config.ledger_path).map_err(|error| {
    format!(
      "failed to read ledger {}: {error}",
      config.ledger_path.display()
    )
  })?;
  let result_text = fs::read_to_string(&config.results_path).map_err(|error| {
    format!(
      "failed to read results {}: {error}",
      config.results_path.display()
    )
  })?;
  let ledger = parse_ledger_csv(&ledger_text)?;

  if let Some(as_of_date) = config.as_of_date.as_deref() {
    let expired_entries = collect_expired_entries(&ledger, as_of_date);

    if !expired_entries.is_empty() {
      let first = &expired_entries[0].key;

      return Err(format!(
        "expired xfail entries are present for as-of {as_of_date}: {}/{}/{} (+{} more)",
        first.suite,
        first.case_id,
        first.target,
        expired_entries.len().saturating_sub(1)
      ));
    }
  }

  let raw_results = parse_results_csv(&result_text)?;
  let classified = classify_results(&ledger, &raw_results)?;
  let summary = summarize(&classified);

  if summary.fail > 0 {
    return Err("non-xfail failures are present".to_string());
  }

  if config.strict_xpass && summary.xpass > 0 {
    return Err("strict-xpass mode: xpass cases are present".to_string());
  }

  Ok(Some(summary))
}

fn parse_args(args: &[String]) -> Result<Action, String> {
  let mut ledger_path = Some(PathBuf::from(DEFAULT_LEDGER_PATH));
  let mut results_path = None;
  let mut seen_ledger_argument = false;
  let mut seen_results_argument = false;
  let mut seen_strict_xpass_argument = false;
  let mut strict_xpass = false;
  let mut as_of_date = None;
  let mut index = 0;

  while index < args.len() {
    let argument = args[index].as_str();

    match argument {
      "-h" | "--help" => {
        return Ok(Action::Help);
      }
      "--" => {
        if let Some(unknown) = args.get(index + 1) {
          return Err(format!("unknown argument: {unknown}"));
        }

        break;
      }
      "--ledger" => {
        if seen_ledger_argument {
          return Err("duplicate --ledger argument".to_string());
        }

        let next_index = index + 1;
        let (value_index, value) = split_option_value(args, "--ledger", next_index)?;

        ledger_path = Some(PathBuf::from(value));
        seen_ledger_argument = true;
        index = value_index;
      }
      _ if argument.starts_with("--ledger=") => {
        if seen_ledger_argument {
          return Err("duplicate --ledger argument".to_string());
        }

        let value = equals_option_value(argument, "--ledger")?;

        ledger_path = Some(PathBuf::from(value));
        seen_ledger_argument = true;
      }
      "--results" => {
        if seen_results_argument {
          return Err("duplicate --results argument".to_string());
        }

        let next_index = index + 1;
        let (value_index, value) = split_option_value(args, "--results", next_index)?;

        results_path = Some(PathBuf::from(value));
        seen_results_argument = true;
        index = value_index;
      }
      _ if argument.starts_with("--results=") => {
        if seen_results_argument {
          return Err("duplicate --results argument".to_string());
        }

        let value = equals_option_value(argument, "--results")?;

        results_path = Some(PathBuf::from(value));
        seen_results_argument = true;
      }
      _ if argument.starts_with("--strict-xpass=") => {
        let value = match equals_option_value(argument, "--strict-xpass") {
          Ok(value) => value,
          Err(error) if error.contains("empty value is not allowed") => {
            return Err(
              "--strict-xpass does not take a value: empty value is not allowed".to_string(),
            );
          }
          Err(_) => return Err("--strict-xpass does not take a value".to_string()),
        };

        return Err(format!("--strict-xpass does not take a value: `{value}`"));
      }
      "--strict-xpass" => {
        if seen_strict_xpass_argument {
          return Err("duplicate --strict-xpass argument".to_string());
        }

        if let Some(next) = args.get(index + 1) {
          if next == "--" {
            if let Some(value) = args.get(index + 2) {
              if value.is_empty() {
                return Err(
                  "--strict-xpass does not take a value: empty value is not allowed".to_string(),
                );
              }

              return Err(format!("--strict-xpass does not take a value: `{value}`"));
            }
          } else if next.is_empty() {
            return Err(
              "--strict-xpass does not take a value: empty value is not allowed".to_string(),
            );
          } else if !is_option_like_token(next) {
            return Err(format!("--strict-xpass does not take a value: `{next}`"));
          }
        }

        strict_xpass = true;
        seen_strict_xpass_argument = true;
      }
      "--as-of" => {
        if as_of_date.is_some() {
          return Err("duplicate --as-of argument".to_string());
        }

        let next_index = index + 1;
        let (value_index, value) = split_option_value(args, "--as-of", next_index)?;

        validate_expires_date(value)
          .map_err(|error| format!("invalid value for --as-of: {error}"))?;
        as_of_date = Some(value.to_string());
        index = value_index;
      }
      _ if argument.starts_with("--as-of=") => {
        if as_of_date.is_some() {
          return Err("duplicate --as-of argument".to_string());
        }

        let value = equals_option_value(argument, "--as-of")?;

        validate_expires_date(value)
          .map_err(|error| format!("invalid value for --as-of: {error}"))?;
        as_of_date = Some(value.to_string());
      }
      unknown => return Err(format!("unknown argument: {unknown}")),
    }

    index += 1;
  }

  let ledger_path = ledger_path.ok_or_else(|| "missing --ledger".to_string())?;
  let results_path = results_path.ok_or_else(|| "missing --results".to_string())?;

  Ok(Action::Run(Config {
    ledger_path,
    results_path,
    strict_xpass,
    as_of_date,
  }))
}

fn required_option_value<'a>(
  args: &'a [String],
  option_name: &str,
  value_index: usize,
) -> Result<&'a str, String> {
  let value = args
    .get(value_index)
    .ok_or_else(|| format!("missing value for {option_name}"))?;

  if value.is_empty() {
    return Err(format!(
      "missing value for {option_name}: empty value is not allowed"
    ));
  }

  if is_option_like_token(value) {
    return Err(format!(
      "missing value for {option_name}: token `{value}` looks like an option; use {option_name}=<value> or {option_name} -- <value> for values starting with '-'"
    ));
  }

  if is_known_option_token(value) {
    return Err(format!("missing value for {option_name}"));
  }

  Ok(value.as_str())
}

fn split_option_value<'a>(
  args: &'a [String],
  option_name: &str,
  value_index: usize,
) -> Result<(usize, &'a str), String> {
  let value = args
    .get(value_index)
    .ok_or_else(|| format!("missing value for {option_name}"))?;

  if value == "--" {
    let separated_value_index = value_index + 1;
    let separated_value = args.get(separated_value_index).ok_or_else(|| {
      format!("missing value for {option_name}: option separator `--` must be followed by a value")
    })?;

    if separated_value.is_empty() {
      return Err(format!(
        "missing value for {option_name}: empty value is not allowed"
      ));
    }

    return Ok((separated_value_index, separated_value.as_str()));
  }

  let normal_value = required_option_value(args, option_name, value_index)?;

  Ok((value_index, normal_value))
}

fn equals_option_value<'a>(argument: &'a str, option_name: &str) -> Result<&'a str, String> {
  let prefix = format!("{option_name}=");
  let value = argument
    .strip_prefix(prefix.as_str())
    .ok_or_else(|| format!("unknown argument: {argument}"))?;

  if value.is_empty() {
    return Err(format!(
      "missing value for {option_name}: empty value is not allowed"
    ));
  }

  Ok(value)
}

fn is_known_option_token(token: &str) -> bool {
  matches!(
    token,
    "-h" | "--help" | "--ledger" | "--results" | "--strict-xpass" | "--as-of"
  ) || token.starts_with("--ledger=")
    || token.starts_with("--results=")
    || token.starts_with("--strict-xpass=")
    || token.starts_with("--as-of=")
}

fn is_option_like_token(token: &str) -> bool {
  token.starts_with('-') && token != "-"
}

fn print_usage() {
  println!(
    "Usage: cargo run --release --bin xfail_ledger -- [--ledger PATH] --results PATH [--strict-xpass] [--as-of YYYY-MM-DD]"
  );
  println!("  --ledger defaults to {DEFAULT_LEDGER_PATH}");
}

fn parse_ledger_csv(input: &str) -> Result<Vec<LedgerEntry>, String> {
  if input.trim().is_empty() {
    return Err("ledger file is empty".to_string());
  }

  let records = parse_csv_records(input).map_err(|error| format!("invalid ledger CSV: {error}"))?;
  let (header_record, data_records) = records
    .split_first()
    .ok_or_else(|| "ledger file is empty".to_string())?;

  if !header_matches(&header_record.fields, &LEDGER_HEADER) {
    return Err("ledger CSV header must be suite,case,target,reason,expires".to_string());
  }

  let mut entries = Vec::new();
  let mut keys = BTreeSet::new();

  for record in data_records {
    let line_no = record.line_no;
    let fields = &record.fields;

    if fields.len() != 5 {
      return Err(format!("ledger line {line_no} must have 5 CSV fields"));
    }

    let suite = fields[0].trim();
    let case_id = fields[1].trim();
    let target = fields[2].trim();
    let reason = fields[3].trim();
    let expires = fields[4].trim();

    if suite.is_empty() || case_id.is_empty() || target.is_empty() || reason.is_empty() {
      return Err(format!(
        "ledger line {line_no} must provide suite/case/target/reason"
      ));
    }

    let key = CaseKey {
      suite: suite.to_string(),
      case_id: case_id.to_string(),
      target: target.to_string(),
    };

    if !keys.insert(key.clone()) {
      return Err(format!(
        "duplicate ledger key at line {line_no}: {suite}/{case_id}/{target}"
      ));
    }

    entries.push(LedgerEntry {
      key,
      reason: reason.to_string(),
      expires: if expires.is_empty() {
        None
      } else {
        validate_expires_date(expires)
          .map_err(|error| format!("ledger line {line_no} invalid expires: {error}"))?;
        Some(expires.to_string())
      },
    });
  }

  Ok(entries)
}

fn parse_results_csv(input: &str) -> Result<Vec<RawResult>, String> {
  if input.trim().is_empty() {
    return Err("result file is empty".to_string());
  }

  let records = parse_csv_records(input).map_err(|error| format!("invalid result CSV: {error}"))?;
  let (header_record, data_records) = records
    .split_first()
    .ok_or_else(|| "result file is empty".to_string())?;

  if !header_matches(&header_record.fields, &RESULT_HEADER) {
    return Err("result CSV header must be suite,case,target,status".to_string());
  }

  let mut entries = Vec::new();
  let mut keys = BTreeSet::new();

  for record in data_records {
    let line_no = record.line_no;
    let fields = &record.fields;

    if fields.len() != 4 {
      return Err(format!("result line {line_no} must have 4 CSV fields"));
    }

    let suite = fields[0].trim();
    let case_id = fields[1].trim();
    let target = fields[2].trim();
    let raw_status = fields[3].trim();

    if suite.is_empty() || case_id.is_empty() || target.is_empty() {
      return Err(format!(
        "result line {line_no} must provide suite/case/target"
      ));
    }

    let key = CaseKey {
      suite: suite.to_string(),
      case_id: case_id.to_string(),
      target: target.to_string(),
    };

    if !keys.insert(key.clone()) {
      return Err(format!(
        "duplicate result key at line {line_no}: {suite}/{case_id}/{target}"
      ));
    }

    let status = parse_raw_status(raw_status)
      .ok_or_else(|| format!("result line {line_no} has unknown status: {raw_status}"))?;

    entries.push(RawResult { key, status });
  }

  Ok(entries)
}

fn classify_results(
  ledger: &[LedgerEntry],
  raw_results: &[RawResult],
) -> Result<Vec<ClassifiedResult>, String> {
  if raw_results.is_empty() {
    return Err("cannot classify empty results".to_string());
  }

  let mut xfail_keys = BTreeMap::new();

  for entry in ledger {
    if xfail_keys
      .insert(entry.key.clone(), entry.reason.clone())
      .is_some()
    {
      return Err(format!(
        "duplicate ledger key in classify input: {}/{}/{}",
        entry.key.suite, entry.key.case_id, entry.key.target
      ));
    }
  }

  let mut output = Vec::with_capacity(raw_results.len());

  for raw in raw_results {
    let status = if xfail_keys.contains_key(&raw.key) {
      match raw.status {
        RawStatus::Pass => FinalStatus::XPass,
        RawStatus::Fail => FinalStatus::XFail,
        RawStatus::Skip => FinalStatus::Skip,
        RawStatus::Timeout => FinalStatus::Timeout,
      }
    } else {
      raw_status_to_final(raw.status)
    };

    output.push(ClassifiedResult {
      key: raw.key.clone(),
      status,
    });
  }

  Ok(output)
}

fn summarize(results: &[ClassifiedResult]) -> Summary {
  let mut summary = Summary::empty();

  for result in results {
    match result.status {
      FinalStatus::Pass => summary.pass += 1,
      FinalStatus::Fail => summary.fail += 1,
      FinalStatus::Skip => summary.skip += 1,
      FinalStatus::Timeout => summary.timeout += 1,
      FinalStatus::XFail => summary.xfail += 1,
      FinalStatus::XPass => summary.xpass += 1,
    }
  }

  summary
}

fn header_matches(fields: &[String], expected: &[&str]) -> bool {
  if fields.len() != expected.len() {
    return false;
  }

  fields
    .iter()
    .zip(expected.iter())
    .all(|(actual, expected_field)| actual.trim() == *expected_field)
}

fn parse_csv_records(input: &str) -> Result<Vec<CsvRecord>, String> {
  let mut records = Vec::new();
  let mut fields = Vec::new();
  let mut field = String::new();
  let mut in_quotes = false;
  let mut just_closed_quote = false;
  let mut chars = input.chars().peekable();
  let mut line_no = 1usize;
  let mut record_line_no = 1usize;

  while let Some(ch) = chars.next() {
    if in_quotes {
      if ch == '"' {
        if chars.peek() == Some(&'"') {
          field.push('"');

          let _ = chars.next();
        } else {
          in_quotes = false;
          just_closed_quote = true;
        }
      } else if ch == '\r' {
        if chars.peek() == Some(&'\n') {
          let _ = chars.next();

          field.push('\n');
          line_no += 1;
        } else {
          field.push('\r');
        }
      } else {
        if ch == '\n' {
          line_no += 1;
        }

        field.push(ch);
      }

      continue;
    }

    match ch {
      ',' => {
        fields.push(std::mem::take(&mut field));
        just_closed_quote = false;
      }
      '\n' => {
        fields.push(std::mem::take(&mut field));

        if is_blank_record(&fields) {
          fields.clear();
        } else {
          records.push(CsvRecord {
            fields: std::mem::take(&mut fields),
            line_no: record_line_no,
          });
        }

        just_closed_quote = false;
        line_no += 1;
        record_line_no = line_no;
      }
      '\r' if chars.peek() == Some(&'\n') => {}
      '"' => {
        if field.is_empty() && !just_closed_quote {
          in_quotes = true;
        } else {
          return Err(format!(
            "line {line_no}: quote must start at beginning of field"
          ));
        }
      }
      _ if just_closed_quote => {
        if !ch.is_ascii_whitespace() {
          return Err(format!(
            "line {line_no}: unexpected character after quoted field"
          ));
        }
      }
      _ => field.push(ch),
    }
  }

  if in_quotes {
    return Err(format!("line {record_line_no}: unterminated quoted field"));
  }

  fields.push(field);

  if !is_blank_record(&fields) {
    records.push(CsvRecord {
      fields,
      line_no: record_line_no,
    });
  }

  Ok(records)
}

fn is_blank_record(fields: &[String]) -> bool {
  fields.len() == 1 && fields[0].trim().is_empty()
}

fn collect_expired_entries<'a>(
  ledger: &'a [LedgerEntry],
  as_of_date: &str,
) -> Vec<&'a LedgerEntry> {
  ledger
    .iter()
    .filter(|entry| {
      entry
        .expires
        .as_deref()
        .is_some_and(|expires| expires < as_of_date)
    })
    .collect()
}

fn validate_expires_date(text: &str) -> Result<(), String> {
  let mut parts = text.split('-');
  let year = parts
    .next()
    .ok_or_else(|| "expected YYYY-MM-DD".to_string())?;
  let month = parts
    .next()
    .ok_or_else(|| "expected YYYY-MM-DD".to_string())?;
  let day = parts
    .next()
    .ok_or_else(|| "expected YYYY-MM-DD".to_string())?;

  if parts.next().is_some() {
    return Err("expected YYYY-MM-DD".to_string());
  }

  if year.len() != 4 || month.len() != 2 || day.len() != 2 {
    return Err("expected YYYY-MM-DD".to_string());
  }

  if !year.chars().all(|ch| ch.is_ascii_digit())
    || !month.chars().all(|ch| ch.is_ascii_digit())
    || !day.chars().all(|ch| ch.is_ascii_digit())
  {
    return Err("expected YYYY-MM-DD digits".to_string());
  }

  let month_value: u32 = month
    .parse()
    .map_err(|_| "expected numeric month".to_string())?;
  let day_value: u32 = day
    .parse()
    .map_err(|_| "expected numeric day".to_string())?;
  let year_value: u32 = year
    .parse()
    .map_err(|_| "expected numeric year".to_string())?;

  if !(1..=12).contains(&month_value) {
    return Err("month must be in 01..12".to_string());
  }

  let max_day = days_in_month(year_value, month_value);

  if day_value == 0 || day_value > max_day {
    return Err(format!("day must be in 01..{max_day:02}"));
  }

  Ok(())
}

const fn days_in_month(year: u32, month: u32) -> u32 {
  match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 if is_leap_year(year) => 29,
    2 => 28,
    _ => 0,
  }
}

const fn is_leap_year(year: u32) -> bool {
  (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

const fn parse_raw_status(status: &str) -> Option<RawStatus> {
  if status.eq_ignore_ascii_case("pass") {
    return Some(RawStatus::Pass);
  }

  if status.eq_ignore_ascii_case("fail") {
    return Some(RawStatus::Fail);
  }

  if status.eq_ignore_ascii_case("skip") {
    return Some(RawStatus::Skip);
  }

  if status.eq_ignore_ascii_case("timeout") {
    return Some(RawStatus::Timeout);
  }

  None
}

const fn raw_status_to_final(status: RawStatus) -> FinalStatus {
  match status {
    RawStatus::Pass => FinalStatus::Pass,
    RawStatus::Fail => FinalStatus::Fail,
    RawStatus::Skip => FinalStatus::Skip,
    RawStatus::Timeout => FinalStatus::Timeout,
  }
}

#[cfg(test)]
mod tests {
  use super::{
    Action, CaseKey, ClassifiedResult, FinalStatus, LedgerEntry, RawResult, RawStatus,
    classify_results, collect_expired_entries, parse_args, parse_ledger_csv, parse_results_csv,
    summarize,
  };

  fn test_key(case_id: &str, target: &str) -> CaseKey {
    CaseKey {
      suite: "libc-test".to_string(),
      case_id: case_id.to_string(),
      target: target.to_string(),
    }
  }

  #[test]
  fn parse_ledger_csv_rejects_duplicate_keys() {
    let input = "\
suite,case,target,reason,expires
libc-test,math/pow,x86_64-unknown-linux-gnu,known bug,
libc-test,math/pow,x86_64-unknown-linux-gnu,still failing,
";
    let error = parse_ledger_csv(input).expect_err("duplicate key must be rejected");

    assert!(error.contains("duplicate"), "unexpected error: {error}");
  }

  #[test]
  fn parse_ledger_csv_accepts_quoted_reason_with_comma() {
    let input = "\
suite,case,target,reason,expires
libc-test,math/pow,x86_64-unknown-linux-gnu,\"known issue, tracks upstream\",
";
    let entries = parse_ledger_csv(input).expect("quoted CSV should parse");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].reason, "known issue, tracks upstream");
  }

  #[test]
  fn parse_ledger_csv_accepts_multiline_quoted_reason() {
    let input = "\
suite,case,target,reason,expires
libc-test,math/pow,x86_64-unknown-linux-gnu,\"known issue,
tracks upstream\",
";
    let entries = parse_ledger_csv(input).expect("multiline quoted CSV should parse");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].reason, "known issue,\ntracks upstream");
  }

  #[test]
  fn parse_ledger_csv_rejects_invalid_expires_date() {
    let input = "\
suite,case,target,reason,expires
libc-test,math/pow,x86_64-unknown-linux-gnu,known issue,2026-13-01
";
    let error = parse_ledger_csv(input).expect_err("invalid expires date must fail");

    assert!(
      error.contains("expires"),
      "unexpected parse error message: {error}"
    );
  }

  #[test]
  fn parse_ledger_csv_accepts_valid_expires_date() {
    let input = "\
suite,case,target,reason,expires
libc-test,math/pow,x86_64-unknown-linux-gnu,known issue,2028-02-29
";
    let entries = parse_ledger_csv(input).expect("valid expires date should parse");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].expires.as_deref(), Some("2028-02-29"));
  }

  #[test]
  fn parse_results_csv_accepts_quoted_case_id_with_comma() {
    let input = "\
suite,case,target,status
ltp,\"math,vector\",x86_64-unknown-linux-gnu,FAIL
";
    let entries = parse_results_csv(input).expect("quoted CSV should parse");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key.case_id, "math,vector");
    assert_eq!(entries[0].status, RawStatus::Fail);
  }

  #[test]
  fn parse_results_csv_accepts_multiline_quoted_case_id() {
    let input = "\
suite,case,target,status
ltp,\"math,
vector\",x86_64-unknown-linux-gnu,FAIL
";
    let entries = parse_results_csv(input).expect("multiline quoted CSV should parse");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key.case_id, "math,\nvector");
    assert_eq!(entries[0].status, RawStatus::Fail);
  }

  #[test]
  fn classify_results_maps_fail_and_pass_against_ledger_entry() {
    let ledger = vec![LedgerEntry {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      reason: "known failure".to_string(),
      expires: None,
    }];
    let raw_results = vec![
      RawResult {
        key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
        status: RawStatus::Fail,
      },
      RawResult {
        key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
        status: RawStatus::Pass,
      },
    ];
    let classified = classify_results(&ledger, &raw_results).expect("classification should work");

    assert_eq!(classified[0].status, FinalStatus::XFail);
    assert_eq!(classified[1].status, FinalStatus::XPass);
  }

  #[test]
  fn classify_results_keeps_skip_for_ledger_entry() {
    let ledger = vec![LedgerEntry {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      reason: "known intermittent".to_string(),
      expires: None,
    }];
    let raw_results = vec![RawResult {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      status: RawStatus::Skip,
    }];
    let classified = classify_results(&ledger, &raw_results).expect("classification should work");

    assert_eq!(classified[0].status, FinalStatus::Skip);
  }

  #[test]
  fn classify_results_keeps_timeout_for_ledger_entry() {
    let ledger = vec![LedgerEntry {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      reason: "known intermittent".to_string(),
      expires: None,
    }];
    let raw_results = vec![RawResult {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      status: RawStatus::Timeout,
    }];
    let classified = classify_results(&ledger, &raw_results).expect("classification should work");

    assert_eq!(classified[0].status, FinalStatus::Timeout);
  }

  #[test]
  fn classify_results_does_not_match_when_target_differs() {
    let ledger = vec![LedgerEntry {
      key: test_key("math/pow", "aarch64-unknown-linux-gnu"),
      reason: "different target".to_string(),
      expires: None,
    }];
    let raw_results = vec![RawResult {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      status: RawStatus::Fail,
    }];
    let classified = classify_results(&ledger, &raw_results).expect("classification should work");

    assert_eq!(classified[0].status, FinalStatus::Fail);
  }

  #[test]
  fn classify_results_rejects_empty_results_input() {
    let ledger = vec![LedgerEntry {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      reason: "known failure".to_string(),
      expires: None,
    }];
    let raw_results = Vec::new();
    let error = classify_results(&ledger, &raw_results).expect_err("empty results must fail");

    assert!(error.contains("cannot classify empty results"));
  }

  #[test]
  fn classify_results_rejects_duplicate_ledger_keys_in_input() {
    let duplicated_key = test_key("math/pow", "x86_64-unknown-linux-gnu");
    let ledger = vec![
      LedgerEntry {
        key: duplicated_key.clone(),
        reason: "first".to_string(),
        expires: None,
      },
      LedgerEntry {
        key: duplicated_key,
        reason: "second".to_string(),
        expires: None,
      },
    ];
    let raw_results = vec![RawResult {
      key: test_key("math/pow", "x86_64-unknown-linux-gnu"),
      status: RawStatus::Fail,
    }];
    let error =
      classify_results(&ledger, &raw_results).expect_err("duplicate ledger keys must fail");

    assert!(error.contains("duplicate ledger key in classify input"));
  }

  #[test]
  fn summarize_counts_all_final_statuses() {
    let results = vec![
      ClassifiedResult {
        key: test_key("a", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::Pass,
      },
      ClassifiedResult {
        key: test_key("b", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::Fail,
      },
      ClassifiedResult {
        key: test_key("c", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::Skip,
      },
      ClassifiedResult {
        key: test_key("d", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::Timeout,
      },
      ClassifiedResult {
        key: test_key("e", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::XFail,
      },
      ClassifiedResult {
        key: test_key("f", "x86_64-unknown-linux-gnu"),
        status: FinalStatus::XPass,
      },
    ];
    let summary = summarize(&results);

    assert_eq!(summary.pass, 1);
    assert_eq!(summary.fail, 1);
    assert_eq!(summary.skip, 1);
    assert_eq!(summary.timeout, 1);
    assert_eq!(summary.xfail, 1);
    assert_eq!(summary.xpass, 1);
  }

  #[test]
  fn parse_results_csv_rejects_unknown_status() {
    let input = "\
suite,case,target,status
libc-test,math/pow,x86_64-unknown-linux-gnu,flaky
";
    let error = parse_results_csv(input).expect_err("unknown status must be rejected");

    assert!(
      error.contains("status"),
      "unexpected parse error message: {error}"
    );
  }

  #[test]
  fn parse_args_supports_help_mode() {
    let args = vec!["--help".to_string()];
    let action = parse_args(&args).expect("help should parse");

    assert_eq!(action, Action::Help);
  }

  #[test]
  fn parse_args_supports_short_help_mode() {
    let args = vec!["-h".to_string()];
    let action = parse_args(&args).expect("short help should parse");

    assert_eq!(action, Action::Help);
  }

  #[test]
  fn parse_args_help_ignores_following_unknown_argument() {
    let args = vec!["--help".to_string(), "--bogus".to_string()];
    let action = parse_args(&args).expect("help should short-circuit argument parsing");

    assert_eq!(action, Action::Help);
  }

  #[test]
  fn parse_args_short_help_ignores_following_unknown_argument() {
    let args = vec!["-h".to_string(), "--bogus".to_string()];
    let action = parse_args(&args).expect("short help should short-circuit argument parsing");

    assert_eq!(action, Action::Help);
  }

  #[test]
  fn parse_args_rejects_unknown_argument_before_help() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--bogus".to_string(),
      "--help".to_string(),
    ];
    let error = parse_args(&args).expect_err("unknown argument before help must fail");

    assert!(error.contains("unknown argument"));
    assert!(error.contains("--bogus"));
  }

  #[test]
  fn parse_args_rejects_unknown_split_style_argument() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--bogus".to_string(),
    ];
    let error = parse_args(&args).expect_err("unknown split-style argument must fail");

    assert!(error.contains("unknown argument"));
    assert!(error.contains("--bogus"));
  }

  #[test]
  fn parse_args_rejects_unknown_equals_style_argument() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--bogus=value".to_string(),
    ];
    let error = parse_args(&args).expect_err("unknown equals-style argument must fail");

    assert!(error.contains("unknown argument"));
    assert!(error.contains("--bogus=value"));
  }

  #[test]
  fn parse_args_accepts_trailing_end_of_options_separator() {
    let args = vec!["--results=/tmp/results.csv".to_string(), "--".to_string()];
    let action = parse_args(&args).expect("trailing end-of-options separator should be ignored");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(
      config.ledger_path,
      std::path::PathBuf::from("docs/conformance/xfail-ledger.csv")
    );
    assert_eq!(
      config.results_path,
      std::path::PathBuf::from("/tmp/results.csv")
    );
    assert_eq!(config.as_of_date, None);
    assert!(!config.strict_xpass);
  }

  #[test]
  fn parse_args_rejects_tokens_after_end_of_options_separator() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--".to_string(),
      "unexpected".to_string(),
    ];
    let error = parse_args(&args).expect_err("positional tokens after end-of-options must fail");

    assert!(error.contains("unknown argument"));
    assert!(error.contains("unexpected"));
  }

  #[test]
  fn parse_args_accepts_trailing_separator_after_strict_xpass() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      "--".to_string(),
    ];
    let action =
      parse_args(&args).expect("trailing separator after strict-xpass should be accepted");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(
      config.results_path,
      std::path::PathBuf::from("/tmp/results.csv")
    );
    assert!(config.strict_xpass);
  }

  #[test]
  fn parse_args_uses_default_ledger_path() {
    let args = vec!["--results".to_string(), "/tmp/results.csv".to_string()];
    let action = parse_args(&args).expect("args should parse");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(
      config.ledger_path,
      std::path::PathBuf::from("docs/conformance/xfail-ledger.csv")
    );
    assert_eq!(
      config.results_path,
      std::path::PathBuf::from("/tmp/results.csv")
    );
  }

  #[test]
  fn parse_args_accepts_as_of_date() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "2026-03-04".to_string(),
    ];
    let action = parse_args(&args).expect("args with --as-of should parse");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(config.as_of_date.as_deref(), Some("2026-03-04"));
  }

  #[test]
  fn parse_args_rejects_invalid_as_of_date() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "2026-13-01".to_string(),
    ];
    let error = parse_args(&args).expect_err("invalid --as-of must fail");

    assert!(error.contains("--as-of"));
  }

  #[test]
  fn parse_args_rejects_invalid_equals_style_as_of_date() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--as-of=2026-13-01".to_string(),
    ];
    let error = parse_args(&args).expect_err("invalid equals-style --as-of must fail");

    assert!(error.contains("--as-of"));
  }

  #[test]
  fn parse_args_rejects_duplicate_as_of_date() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "2026-03-04".to_string(),
      "--as-of".to_string(),
      "2026-03-05".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --as-of must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--as-of"));
  }

  #[test]
  fn parse_args_rejects_duplicate_as_of_across_equals_and_split_forms() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--as-of=2026-03-04".to_string(),
      "--as-of".to_string(),
      "2026-03-05".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --as-of across forms must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--as-of"));
  }

  #[test]
  fn parse_args_rejects_duplicate_as_of_across_split_and_equals_forms() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "2026-03-04".to_string(),
      "--as-of=2026-03-05".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --as-of across forms must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--as-of"));
  }

  #[test]
  fn parse_args_rejects_duplicate_results_argument() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results-a.csv".to_string(),
      "--results".to_string(),
      "/tmp/results-b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --results must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--results"));
  }

  #[test]
  fn parse_args_rejects_duplicate_results_across_split_and_equals_forms() {
    let args = vec![
      "--results".to_string(),
      "/tmp/a.csv".to_string(),
      "--results=/tmp/b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --results across forms must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--results"));
  }

  #[test]
  fn parse_args_rejects_duplicate_ledger_argument() {
    let args = vec![
      "--ledger".to_string(),
      "/tmp/ledger-a.csv".to_string(),
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--ledger".to_string(),
      "/tmp/ledger-b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --ledger must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--ledger"));
  }

  #[test]
  fn parse_args_rejects_duplicate_ledger_across_equals_and_split_forms() {
    let args = vec![
      "--ledger=/tmp/ledger-a.csv".to_string(),
      "--results=/tmp/results.csv".to_string(),
      "--ledger".to_string(),
      "/tmp/ledger-b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --ledger across forms must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--ledger"));
  }

  #[test]
  fn parse_args_rejects_duplicate_ledger_across_split_and_equals_forms() {
    let args = vec![
      "--ledger".to_string(),
      "/tmp/ledger-a.csv".to_string(),
      "--results=/tmp/results.csv".to_string(),
      "--ledger=/tmp/ledger-b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --ledger across forms must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--ledger"));
  }

  #[test]
  fn parse_args_rejects_duplicate_strict_xpass_flag() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      "--strict-xpass".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --strict-xpass must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--strict-xpass"));
  }

  #[test]
  fn parse_args_rejects_missing_results_value_when_next_is_option() {
    let args = vec!["--results".to_string(), "--strict-xpass".to_string()];
    let error = parse_args(&args).expect_err("missing --results value must fail");

    assert!(error.contains("missing value for --results"));
  }

  #[test]
  fn parse_args_rejects_missing_results_value_when_next_is_unknown_option_token() {
    let args = vec!["--results".to_string(), "--bogus".to_string()];
    let error = parse_args(&args)
      .expect_err("missing --results value must fail when next token looks like an option");

    assert!(error.contains("missing value for --results"));
  }

  #[test]
  fn parse_args_reports_hint_for_option_like_results_value_token() {
    let args = vec!["--results".to_string(), "--bogus".to_string()];
    let error = parse_args(&args).expect_err("option-like value token must be rejected");

    assert!(error.contains("use --results=<value>"));
  }

  #[test]
  fn parse_args_reports_separator_hint_for_option_like_results_value_token() {
    let args = vec!["--results".to_string(), "--bogus".to_string()];
    let error = parse_args(&args).expect_err("option-like value token must be rejected");

    assert!(
      error.contains("--results -- <value>"),
      "unexpected error message: {error}"
    );
  }

  #[test]
  fn parse_args_reports_hint_for_known_option_used_as_results_value() {
    let args = vec!["--results".to_string(), "--strict-xpass".to_string()];
    let error =
      parse_args(&args).expect_err("known option token used as --results value must report hint");

    assert!(error.contains("missing value for --results"));
    assert!(error.contains("use --results=<value>"));
  }

  #[test]
  fn parse_args_accepts_option_like_results_value_after_separator() {
    let args = vec![
      "--results".to_string(),
      "--".to_string(),
      "--report.csv".to_string(),
    ];
    let action =
      parse_args(&args).expect("option-like --results value should parse when separated by `--`");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(
      config.results_path,
      std::path::PathBuf::from("--report.csv")
    );
  }

  #[test]
  fn parse_args_rejects_separator_without_following_results_value() {
    let args = vec!["--results".to_string(), "--".to_string()];
    let error = parse_args(&args).expect_err("separator without following value must fail");

    assert!(error.contains("missing value for --results"));
    assert!(error.contains("option separator"));
  }

  #[test]
  fn parse_args_accepts_option_like_ledger_value_after_separator() {
    let args = vec![
      "--ledger".to_string(),
      "--".to_string(),
      "--ledger.csv".to_string(),
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
    ];
    let action =
      parse_args(&args).expect("option-like --ledger value should parse when separated by `--`");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(config.ledger_path, std::path::PathBuf::from("--ledger.csv"));
  }

  #[test]
  fn parse_args_rejects_separator_without_following_ledger_value() {
    let args = vec!["--ledger".to_string(), "--".to_string()];
    let error = parse_args(&args).expect_err("separator without following ledger value must fail");

    assert!(error.contains("missing value for --ledger"));
    assert!(error.contains("option separator"));
  }

  #[test]
  fn parse_args_accepts_as_of_value_after_separator() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "--".to_string(),
      "2026-03-04".to_string(),
    ];
    let action = parse_args(&args).expect("--as-of value after separator should parse");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(config.as_of_date.as_deref(), Some("2026-03-04"));
  }

  #[test]
  fn parse_args_rejects_separator_without_following_as_of_value() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "--".to_string(),
    ];
    let error = parse_args(&args).expect_err("separator without following as-of value must fail");

    assert!(error.contains("missing value for --as-of"));
    assert!(error.contains("option separator"));
  }

  #[test]
  fn parse_args_rejects_empty_split_results_value() {
    let args = vec!["--results".to_string(), String::new()];
    let error = parse_args(&args).expect_err("empty split-form --results value must fail");

    assert!(error.contains("missing value for --results"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_split_results_value() {
    let args = vec!["--results".to_string(), String::new()];
    let error =
      parse_args(&args).expect_err("empty split-form --results should report explicit reason");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_empty_split_ledger_value() {
    let args = vec![
      "--ledger".to_string(),
      String::new(),
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("empty split-form --ledger value must fail");

    assert!(error.contains("missing value for --ledger"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_split_ledger_value() {
    let args = vec![
      "--ledger".to_string(),
      String::new(),
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
    ];
    let error =
      parse_args(&args).expect_err("empty split-form --ledger should report explicit reason");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_empty_split_as_of_value() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      String::new(),
    ];
    let error = parse_args(&args).expect_err("empty split-form --as-of value must fail");

    assert!(error.contains("missing value for --as-of"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_split_as_of_value() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      String::new(),
    ];
    let error =
      parse_args(&args).expect_err("empty split-form --as-of should report explicit reason");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_missing_ledger_value_when_next_is_option() {
    let args = vec![
      "--ledger".to_string(),
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("missing --ledger value must fail");

    assert!(error.contains("missing value for --ledger"));
  }

  #[test]
  fn parse_args_rejects_missing_ledger_value_when_next_is_equals_style_option() {
    let args = vec![
      "--ledger".to_string(),
      "--results=/tmp/results.csv".to_string(),
    ];
    let error = parse_args(&args)
      .expect_err("missing --ledger value must fail when next token is equals-style option");

    assert!(error.contains("missing value for --ledger"));
  }

  #[test]
  fn parse_args_rejects_missing_as_of_value_when_next_is_option() {
    let args = vec![
      "--results".to_string(),
      "/tmp/results.csv".to_string(),
      "--as-of".to_string(),
      "--strict-xpass".to_string(),
    ];
    let error = parse_args(&args).expect_err("missing --as-of value must fail");

    assert!(error.contains("missing value for --as-of"));
  }

  #[test]
  fn parse_args_accepts_equals_style_values() {
    let args = vec![
      "--ledger=/tmp/ledger.csv".to_string(),
      "--results=/tmp/results.csv".to_string(),
      "--as-of=2026-03-04".to_string(),
      "--strict-xpass".to_string(),
    ];
    let action = parse_args(&args).expect("equals-style args should parse");
    let Action::Run(config) = action else {
      panic!("expected run action");
    };

    assert_eq!(
      config.ledger_path,
      std::path::PathBuf::from("/tmp/ledger.csv")
    );
    assert_eq!(
      config.results_path,
      std::path::PathBuf::from("/tmp/results.csv")
    );
    assert_eq!(config.as_of_date.as_deref(), Some("2026-03-04"));
    assert!(config.strict_xpass);
  }

  #[test]
  fn parse_args_rejects_empty_equals_style_results_value() {
    let args = vec!["--results=".to_string()];
    let error = parse_args(&args).expect_err("empty equals-style results must fail");

    assert!(error.contains("missing value for --results"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_equals_style_results_value() {
    let args = vec!["--results=".to_string()];
    let error =
      parse_args(&args).expect_err("empty equals-style --results should report explicit reason");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_equals_style_ledger_value() {
    let args = vec![
      "--ledger=".to_string(),
      "--results=/tmp/results.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("empty equals-style --ledger must fail");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_reports_explicit_reason_for_empty_equals_style_as_of_value() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--as-of=".to_string(),
    ];
    let error = parse_args(&args).expect_err("empty equals-style --as-of must fail");

    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_duplicate_results_across_equals_and_split_forms() {
    let args = vec![
      "--results=/tmp/a.csv".to_string(),
      "--results".to_string(),
      "/tmp/b.csv".to_string(),
    ];
    let error = parse_args(&args).expect_err("duplicate --results must fail");

    assert!(error.contains("duplicate"));
    assert!(error.contains("--results"));
  }

  #[test]
  fn parse_args_rejects_strict_xpass_equals_value_with_explicit_error() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass=true".to_string(),
    ];
    let error =
      parse_args(&args).expect_err("equals-style strict-xpass with value must be rejected");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
  }

  #[test]
  fn parse_args_reports_offending_token_for_equals_style_strict_xpass_value() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass=always".to_string(),
    ];
    let error =
      parse_args(&args).expect_err("equals-style strict-xpass value must report token detail");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
    assert!(error.contains("always"));
  }

  #[test]
  fn parse_args_rejects_empty_strict_xpass_equals_value_with_explicit_error() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass=".to_string(),
    ];
    let error = parse_args(&args).expect_err("empty equals-style strict-xpass must be rejected");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
  }

  #[test]
  fn parse_args_reports_empty_reason_for_empty_strict_xpass_equals_value() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass=".to_string(),
    ];
    let error =
      parse_args(&args).expect_err("empty equals-style strict-xpass must report explicit reason");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_split_style_strict_xpass_value_with_explicit_error() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      "true".to_string(),
    ];
    let error = parse_args(&args).expect_err("split-style strict-xpass value must be rejected");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
  }

  #[test]
  fn parse_args_reports_empty_reason_for_split_style_strict_xpass_value() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      String::new(),
    ];
    let error =
      parse_args(&args).expect_err("split-style strict-xpass with empty value must report reason");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
    assert!(error.contains("empty value is not allowed"));
  }

  #[test]
  fn parse_args_rejects_separator_style_strict_xpass_value_with_explicit_error() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      "--".to_string(),
      "unexpected".to_string(),
    ];
    let error = parse_args(&args)
      .expect_err("separator-style strict-xpass value attempt must be rejected explicitly");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
  }

  #[test]
  fn parse_args_reports_offending_token_for_separator_style_strict_xpass_value() {
    let args = vec![
      "--results=/tmp/results.csv".to_string(),
      "--strict-xpass".to_string(),
      "--".to_string(),
      "--as-literal".to_string(),
    ];
    let error =
      parse_args(&args).expect_err("separator-style strict-xpass value must report token");

    assert!(error.contains("--strict-xpass"));
    assert!(error.contains("does not take a value"));
    assert!(error.contains("--as-literal"));
  }

  #[test]
  fn collect_expired_entries_finds_only_dates_before_as_of() {
    let ledger = vec![
      LedgerEntry {
        key: test_key("old", "x86_64-unknown-linux-gnu"),
        reason: "old failure".to_string(),
        expires: Some("2026-03-01".to_string()),
      },
      LedgerEntry {
        key: test_key("equal", "x86_64-unknown-linux-gnu"),
        reason: "current".to_string(),
        expires: Some("2026-03-04".to_string()),
      },
      LedgerEntry {
        key: test_key("none", "x86_64-unknown-linux-gnu"),
        reason: "permanent".to_string(),
        expires: None,
      },
    ];
    let expired = collect_expired_entries(&ledger, "2026-03-04");

    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].key.case_id, "old");
  }
}
