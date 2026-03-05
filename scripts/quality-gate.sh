#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

cd "${REPO_ROOT}"

if command -v timeout >/dev/null 2>&1; then
  TIMEOUT_BIN="timeout"
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT_BIN="gtimeout"
else
  echo "[quality-gate] failed: timeout command not found (tried: timeout, gtimeout)" >&2
  exit 127
fi

PROFILE="pr"
PROFILE_SET="0"
CONTINUE_ON_FAIL="0"
CONTINUE_ON_FAIL_SET="0"
OVERALL_STATUS=0
FAILED_TESTS_PATH="${REPO_ROOT}/docs/failed-tests.md"
FAILURE_ISSUE_ID="${RLIBC_FAILURE_ISSUE_ID:-CI}"
FAILURE_LOG_ENABLED="${RLIBC_FAILURE_LOG_ENABLED:-1}"

append_failure_log() {
  local command="$1"
  local reason="$2"

  if [[ "${FAILURE_LOG_ENABLED}" != "1" ]]; then
    return 0
  fi

  if [[ ! -f "${FAILED_TESTS_PATH}" ]]; then
    return 0
  fi

  printf '%s|%s|%s\n' "${FAILURE_ISSUE_ID}" "${command}" "${reason}" >> "${FAILED_TESTS_PATH}" || true
}

print_usage() {
  cat <<'USAGE'
Usage: bash scripts/quality-gate.sh [--profile <pr|nightly|full>|--profile=<pr|nightly|full>] [--continue-on-fail]

Profiles:
  default: pr when omitted
  pr       fast checks for pull requests and pushes
  nightly  extended checks for scheduled runs
  full     nightly checks + optional conformance adapters

Options:
  --profile <pr|nightly|full>  select quality-gate profile
                     supports --profile <value> and --profile=<value> forms
                     duplicate --profile is rejected for both --profile <value> and --profile=<value> forms
  --continue-on-fail  keep running remaining profile steps and exit non-zero at end if any step failed (flag only; no value)
                     equals-style values are rejected (for example: --continue-on-fail=1)
                     duplicate options are rejected (for example: repeated --profile or --continue-on-fail)
                     duplicate --continue-on-fail is rejected when the flag is repeated
                     default: stop on first failed step
  -h, --help         show this help and exit
  --                end option parsing (positional args are rejected)
USAGE
}

set_profile_value() {
  local candidate="$1"

  if [[ "${PROFILE_SET}" == "1" ]]; then
    echo "[quality-gate] failed: --profile specified multiple times" >&2
    exit 2
  fi

  PROFILE="${candidate}"
  if [[ -z "${PROFILE}" ]]; then
    echo "[quality-gate] failed: --profile requires a non-empty value" >&2
    exit 2
  fi

  if [[ "${PROFILE}" == -* ]]; then
    echo "[quality-gate] failed: --profile value must not start with '-': ${PROFILE}" >&2
    exit 2
  fi

  PROFILE_SET="1"
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --profile)
        shift
        if [[ $# -eq 0 ]]; then
          echo "[quality-gate] failed: --profile requires a value" >&2
          exit 2
        fi
        if [[ "$1" == -* ]]; then
          echo "[quality-gate] failed: --profile requires a value" >&2
          exit 2
        fi

        set_profile_value "$1"
        ;;
      --profile=*)
        set_profile_value "${1#--profile=}"
        ;;
      --continue-on-fail)
        if [[ "${CONTINUE_ON_FAIL_SET}" == "1" ]]; then
          echo "[quality-gate] failed: --continue-on-fail specified multiple times" >&2
          exit 2
        fi

        CONTINUE_ON_FAIL="1"
        CONTINUE_ON_FAIL_SET="1"
        ;;
      --continue-on-fail=*)
        echo "[quality-gate] failed: --continue-on-fail does not take a value: ${1}" >&2
        exit 2
        ;;
      --)
        shift
        break
        ;;
      -h | --help)
        print_usage
        exit 0
        ;;
      -*)
        echo "[quality-gate] failed: unknown argument: $1" >&2
        print_usage
        exit 2
        ;;
      *)
        echo "[quality-gate] failed: unexpected positional argument: $1" >&2
        print_usage
        exit 2
        ;;
    esac
    shift
  done

  if [[ $# -gt 0 ]]; then
    echo "[quality-gate] failed: unexpected positional argument: $1" >&2
    print_usage
    exit 2
  fi
}

run_step() {
  local timeout_seconds="$1"
  shift

  local command_status=0
  local command_string="${TIMEOUT_BIN} ${timeout_seconds}"
  local argument=""

  for argument in "$@"; do
    command_string+=" $(printf '%q' "${argument}")"
  done

  echo "[quality-gate:${PROFILE}] running: ${command_string}"

  set +e
  "${TIMEOUT_BIN}" "${timeout_seconds}" "$@"
  command_status="$?"
  set -e

  if [[ "${command_status}" -eq 0 ]]; then
    return 0
  fi

  if [[ "${command_status}" -eq 124 ]]; then
    append_failure_log "${command_string}" "stuck (timeout exit 124)"
  else
    append_failure_log "${command_string}" "exit code ${command_status}"
  fi

  if [[ "${CONTINUE_ON_FAIL}" == "1" ]]; then
    if [[ "${OVERALL_STATUS}" -eq 0 ]]; then
      OVERALL_STATUS="${command_status}"
    fi

    return 0
  fi

  return "${command_status}"
}

run_pr_profile() {
  run_step 300 cargo test --release --workspace --test quality_gate_ci
  run_step 300 cargo test --release --workspace --test stdlib_process abort_ignored_sigabrt_still_terminates_with_signal_and_skips_atexit_handlers -- --exact --test-threads=1
  run_step 300 cargo test --release --workspace --test stdlib_process abort_caught_sigabrt_runs_handler_then_terminates_with_signal -- --exact --test-threads=1
  run_step 300 cargo run --release -p codestyle-check --bin codestyle_check -- --all --no-clippy
  run_step 300 cargo check --release --workspace
  run_step 300 cargo test --release --workspace --test library_shape
  run_step 300 cargo test --release --workspace --test abi_baseline
}

run_nightly_profile() {
  run_pr_profile
  run_step 300 cargo clippy --release --workspace
  run_step 300 cargo test --release --workspace
  run_step 300 cargo rustc --release --lib --crate-type cdylib
  run_step 120 cargo run --release --bin abi_check -- --golden abi/golden/x86_64-unknown-linux-gnu.abi
}

run_full_profile() {
  run_nightly_profile

  if [[ -x scripts/conformance/libc-test-adapter.sh ]]; then
    if [[ -n "${RLIBC_LIBC_TEST_ROOT:-}" ]]; then
      run_step 900 bash scripts/conformance/libc-test-adapter.sh --profile docs/conformance/libc-test-smoke.txt
    else
      run_step 120 bash scripts/conformance/libc-test-adapter.sh --dry-run --profile docs/conformance/libc-test-smoke.txt
    fi
  else
    echo "[quality-gate:${PROFILE}] skipping optional adapter: scripts/conformance/libc-test-adapter.sh"
  fi

  if [[ -x scripts/conformance/ltp-openposix-adapter.sh ]] && [[ -n "${RLIBC_LTP_SUITE_ROOT:-}" ]]; then
    run_step 1800 bash scripts/conformance/ltp-openposix-adapter.sh --suite "${RLIBC_LTP_SUITE:-ltp}" --suite-root "${RLIBC_LTP_SUITE_ROOT}"
  else
    echo "[quality-gate:${PROFILE}] skipping optional adapter: scripts/conformance/ltp-openposix-adapter.sh"
  fi

  if [[ -f docs/conformance/xfail-ledger.csv ]]; then
    run_step 60 awk -F',' 'NR == 1 { exit $1 == "suite" && $2 == "case" ? 0 : 1 }' docs/conformance/xfail-ledger.csv
  else
    echo "[quality-gate:${PROFILE}] skipping optional adapter: docs/conformance/xfail-ledger.csv"
  fi
}

parse_args "$@"

case "${PROFILE}" in
  pr)
    run_pr_profile
    ;;
  nightly)
    run_nightly_profile
    ;;
  full)
    run_full_profile
    ;;
  *)
    echo "[quality-gate] failed: unsupported profile '${PROFILE}'" >&2
    print_usage
    exit 2
    ;;
esac

if [[ "${OVERALL_STATUS}" -ne 0 ]]; then
  exit "${OVERALL_STATUS}"
fi
