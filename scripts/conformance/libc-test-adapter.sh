#!/usr/bin/env bash
set -euo pipefail

resolve_timeout_bin() {
  if command -v timeout >/dev/null 2>&1; then
    printf '%s\n' "timeout"

    return 0
  fi

  if command -v gtimeout >/dev/null 2>&1; then
    printf '%s\n' "gtimeout"

    return 0
  fi

  echo "[libc-test-adapter] timeout command is required (timeout/gtimeout)" >&2

  return 1
}

run_step() {
  local timeout_seconds="$1"
  shift

  echo "[libc-test-adapter] running: $*"
  "${TIMEOUT_BIN}" "${timeout_seconds}" "$@"
}

trim_ascii_whitespace() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"

  printf '%s' "${value}"
}

print_usage() {
  cat <<'USAGE'
Usage: bash scripts/conformance/libc-test-adapter.sh [--dry-run] [--profile <manifest-path>]

Options:
  --dry-run              Validate adapter inputs and list smoke commands without executing them.
  --profile <path>       Override smoke manifest path (default: docs/conformance/libc-test-smoke.txt).
  -h, --help             Show this help message.

Environment:
  RLIBC_LIBC_TEST_RUNTEST Optional `runtest` command path used to rewrite
                          `runtest` / `./runtest` / `bin/runtest` /
                          `./bin/runtest` manifest entries.
USAGE
}

normalize_runtest_command() {
  local command="$1"

  if [[ "${command}" = /* ]]; then
    printf '%s' "${command}"

    return 0
  fi

  command="${command#./}"
  printf './%s' "${command}"
}

resolve_runtest_command() {
  if [[ -n "${RLIBC_LIBC_TEST_RUNTEST+x}" ]]; then
    local configured_runtest
    configured_runtest="$(trim_ascii_whitespace "${RLIBC_LIBC_TEST_RUNTEST}")"

    if [[ -z "${configured_runtest}" ]]; then
      echo "[libc-test-adapter] RLIBC_LIBC_TEST_RUNTEST must not be empty when set" >&2

      return 1
    fi

    if [[ "${configured_runtest}" =~ [[:space:]] ]]; then
      echo "[libc-test-adapter] RLIBC_LIBC_TEST_RUNTEST must not contain whitespace" >&2

      return 1
    fi

    if [[ ! "${configured_runtest}" =~ ^[A-Za-z0-9_./:-]+$ ]]; then
      echo "[libc-test-adapter] RLIBC_LIBC_TEST_RUNTEST must be a path-like token without shell metacharacters" >&2

      return 1
    fi

    normalize_runtest_command "${configured_runtest}"

    return 0
  fi

  if [[ -n "${LIBC_TEST_ROOT}" ]] && [[ -x "${LIBC_TEST_ROOT}/runtest" ]]; then
    printf '%s' "./runtest"

    return 0
  fi

  if [[ -n "${LIBC_TEST_ROOT}" ]] && [[ -x "${LIBC_TEST_ROOT}/bin/runtest" ]]; then
    printf '%s' "./bin/runtest"

    return 0
  fi

  printf '%s' "./runtest"
}

rewrite_runtest_prefix() {
  local command="$1"
  local runtest_command="$2"

  if [[ "${command}" =~ ^(runtest|./runtest|bin/runtest|./bin/runtest)([[:space:]]+.*)?$ ]]; then
    local command_suffix="${BASH_REMATCH[2]}"
    if [[ -n "${command_suffix}" ]]; then
      command_suffix="$(trim_ascii_whitespace "${command_suffix}")"
      printf '%s %s' "${runtest_command}" "${command_suffix}"

      return 0
    fi

    printf '%s' "${runtest_command}"

    return 0
  fi

  printf '%s' "${command}"
}

command_uses_runtest_prefix() {
  local command="$1"
  [[ "${command}" =~ ^(runtest|./runtest|bin/runtest|./bin/runtest)([[:space:]]+.*)?$ ]]
}

runtest_command_has_arguments() {
  local command="$1"

  if [[ ! "${command}" =~ ^(runtest|./runtest|bin/runtest|./bin/runtest)[[:space:]]+(.+)$ ]]; then
    return 1
  fi

  local command_suffix="${BASH_REMATCH[2]}"
  command_suffix="$(trim_ascii_whitespace "${command_suffix}")"

  if [[ -z "${command_suffix}" ]] || [[ "${command_suffix}" == \#* ]]; then
    return 1
  fi

  if [[ "${command_suffix}" =~ (^|[[:space:]])-w[[:space:]]*$ ]]; then
    return 1
  fi

  local -a suffix_tokens=()
  read -r -a suffix_tokens <<< "${command_suffix}"

  if [[ "${#suffix_tokens[@]}" -eq 0 ]]; then
    return 1
  fi

  local token_count="${#suffix_tokens[@]}"
  local token_index=0
  local saw_workload_flag=0
  local has_unsupported_shell_operators=0

  if runtest_command_uses_unsupported_shell_operators "${command}"; then
    has_unsupported_shell_operators=1
  fi

  while [[ "${token_index}" -lt "${token_count}" ]]; do
    local token="${suffix_tokens[${token_index}]}"

    if [[ "${token}" == "-w="* ]]; then
      return 1
    fi

    if [[ "${token}" == "-w"* ]] && [[ "${token}" != "-w" ]]; then
      return 1
    fi

    if [[ "${token}" != "-w" ]]; then
      if [[ "${token}" == -* ]]; then
        return 1
      fi

      if [[ "${has_unsupported_shell_operators}" -eq 1 ]]; then
        token_index=$((token_index + 1))

        continue
      fi

      return 1
    fi

    saw_workload_flag=1

    local workload_index=$((token_index + 1))

    if [[ "${workload_index}" -ge "${token_count}" ]]; then
      return 1
    fi

    local workload="${suffix_tokens[${workload_index}]}"

    if [[ "${workload}" == -* ]]; then
      return 1
    fi

    if [[ "${workload}" == /* ]]; then
      return 1
    fi

    if [[ "${workload}" == *"//"* ]]; then
      return 1
    fi

    if [[ "${workload}" =~ (^|/)\.(/|$) ]] || [[ "${workload}" =~ (^|/)\.\.(/|$) ]]; then
      return 1
    fi

    token_index=$((workload_index + 1))
  done

  if [[ "${saw_workload_flag}" -eq 0 ]]; then
    return 1
  fi

  return 0
}

runtest_command_uses_unsupported_shell_operators() {
  local command="$1"
  local command_suffix

  if [[ ! "${command}" =~ ^(runtest|./runtest|bin/runtest|./bin/runtest)[[:space:]]+(.+)$ ]]; then
    return 1
  fi

  command_suffix="$(trim_ascii_whitespace "${BASH_REMATCH[2]}")"

  if [[ "${command_suffix}" == *"&&"* ]] || [[ "${command_suffix}" == *"||"* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *'$('* ]] || [[ "${command_suffix}" == *'$'* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *\\* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *\** ]] || [[ "${command_suffix}" == *\?* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"["* ]] || [[ "${command_suffix}" == *"]"* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"{"* ]] || [[ "${command_suffix}" == *"}"* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"("* ]] || [[ "${command_suffix}" == *")"* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"'"* ]] || [[ "${command_suffix}" == *'"'* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *";"* ]] || [[ "${command_suffix}" == *"&"* ]] || [[ "${command_suffix}" == *"|"* ]] || [[ "${command_suffix}" == *"!"* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"<"* ]] || [[ "${command_suffix}" == *">"* ]] || [[ "${command_suffix}" == *'`'* ]]; then
    return 0
  fi

  if [[ "${command_suffix}" == *"#"* ]]; then
    return 0
  fi

  return 1
}

resolve_command_path() {
  local command="$1"

  if [[ "${command}" = /* ]]; then
    printf '%s' "${command}"

    return 0
  fi

  printf '%s/%s' "${LIBC_TEST_ROOT}" "${command#./}"
}

validate_runtest_command() {
  local runtest_command="$1"
  local runtest_path

  runtest_path="$(resolve_command_path "${runtest_command}")"

  if [[ ! -x "${runtest_path}" ]]; then
    echo "[libc-test-adapter] runtest command is not executable: ${runtest_command} (resolved: ${runtest_path})" >&2
    echo "[libc-test-adapter] set RLIBC_LIBC_TEST_RUNTEST to override runtest location" >&2

    return 1
  fi

  return 0
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --dry-run)
        DRY_RUN=1
        ;;
      --profile)
        shift
        if [[ $# -eq 0 ]]; then
          echo "[libc-test-adapter] --profile requires a path argument" >&2

          exit 2
        fi

        MANIFEST_PATH="$1"
        ;;
      -h | --help)
        print_usage

        exit 0
        ;;
      *)
        echo "[libc-test-adapter] unknown argument: $1" >&2
        print_usage

        exit 2
        ;;
    esac
    shift
  done
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DEFAULT_MANIFEST="${REPO_ROOT}/docs/conformance/libc-test-smoke.txt"
MANIFEST_PATH="${RLIBC_LIBC_TEST_SMOKE_MANIFEST:-${DEFAULT_MANIFEST}}"
LIBC_TEST_ROOT="${RLIBC_LIBC_TEST_ROOT:-}"
DRY_RUN=0
TIMEOUT_BIN="$(resolve_timeout_bin)"

parse_args "$@"

if [[ "${MANIFEST_PATH}" != /* ]]; then
  MANIFEST_PATH="${REPO_ROOT}/${MANIFEST_PATH}"
fi

if [[ "${DRY_RUN}" -eq 0 ]] && [[ -z "${LIBC_TEST_ROOT}" ]]; then
  echo "[libc-test-adapter] RLIBC_LIBC_TEST_ROOT is required unless --dry-run is used" >&2

  exit 2
fi

if [[ "${DRY_RUN}" -eq 0 ]] && [[ ! -d "${LIBC_TEST_ROOT}" ]]; then
  echo "[libc-test-adapter] RLIBC_LIBC_TEST_ROOT does not exist: ${LIBC_TEST_ROOT}" >&2

  exit 2
fi

if [[ ! -f "${MANIFEST_PATH}" ]]; then
  echo "[libc-test-adapter] smoke manifest does not exist: ${MANIFEST_PATH}" >&2

  exit 2
fi

if ! RUNTEST_COMMAND="$(resolve_runtest_command)"; then
  exit 2
fi

cd "${REPO_ROOT}"

if [[ "${DRY_RUN}" -eq 0 ]]; then
  run_step 300 cargo rustc --release --lib --crate-type cdylib
  run_step 300 cargo rustc --release --lib --crate-type staticlib
fi

RLIBC_SHARED_LIB="${REPO_ROOT}/target/release/librlibc.so"
RLIBC_STATIC_LIB="${REPO_ROOT}/target/release/librlibc.a"
export RLIBC_SHARED_LIB
export RLIBC_STATIC_LIB

if [[ "${DRY_RUN}" -eq 0 ]] && [[ ! -f "${RLIBC_SHARED_LIB}" ]]; then
  echo "[libc-test-adapter] shared library not found: ${RLIBC_SHARED_LIB}" >&2

  exit 3
fi

if [[ "${DRY_RUN}" -eq 0 ]] && [[ ! -f "${RLIBC_STATIC_LIB}" ]]; then
  echo "[libc-test-adapter] static library not found: ${RLIBC_STATIC_LIB}" >&2

  exit 3
fi

total_cases=0
failed_cases=0
line_number=0
runtest_validated=0
previous_case_id=""
declare -A seen_case_ids=()
declare -A seen_case_lines=()

while IFS= read -r raw_line || [[ -n "${raw_line}" ]]; do
  line_number=$((line_number + 1))
  line="$(trim_ascii_whitespace "${raw_line}")"

  if [[ -z "${line}" ]] || [[ "${line}" == \#* ]]; then
    continue
  fi

  if [[ "${line}" != *"|"* ]]; then
    echo "[libc-test-adapter] invalid smoke entry at line ${line_number} (expected <case-id>|<command>): ${line}" >&2

    exit 2
  fi

  case_id="$(trim_ascii_whitespace "${line%%|*}")"
  case_command="$(trim_ascii_whitespace "${line#*|}")"

  if [[ -z "${case_id}" ]] || [[ -z "${case_command}" ]]; then
    echo "[libc-test-adapter] invalid smoke entry at line ${line_number} (expected <case-id>|<command>): ${line}" >&2

    exit 2
  fi

  if [[ ! "${case_id}" =~ ^[A-Za-z0-9][A-Za-z0-9._:-]*$ ]]; then
    echo "[libc-test-adapter] invalid smoke case id at line ${line_number}: ${case_id}" >&2

    exit 2
  fi

  uses_runtest_prefix=0
  if command_uses_runtest_prefix "${case_command}"; then
    if ! runtest_command_has_arguments "${case_command}"; then
      echo "[libc-test-adapter] runtest smoke command requires explicit arguments at line ${line_number}: ${line}" >&2

      exit 2
    fi

    if runtest_command_uses_unsupported_shell_operators "${case_command}"; then
      echo "[libc-test-adapter] runtest smoke command contains unsupported shell operators at line ${line_number}: ${line}" >&2

      exit 2
    fi

    uses_runtest_prefix=1
    case_command="$(rewrite_runtest_prefix "${case_command}" "${RUNTEST_COMMAND}")"
  fi

  if [[ -n "${seen_case_ids["${case_id}"]+x}" ]]; then
    echo "[libc-test-adapter] duplicate smoke case id: ${case_id} (line ${line_number}, first seen at line ${seen_case_lines["${case_id}"]})" >&2

    exit 2
  fi

  if [[ -n "${previous_case_id}" ]] && [[ "${case_id}" < "${previous_case_id}" ]]; then
    echo "[libc-test-adapter] smoke case ids must be sorted: ${case_id} appeared after ${previous_case_id} (line ${line_number})" >&2

    exit 2
  fi

  previous_case_id="${case_id}"
  seen_case_ids["${case_id}"]=1
  seen_case_lines["${case_id}"]="${line_number}"

  total_cases=$((total_cases + 1))
  echo "[libc-test-adapter] smoke case ${case_id}"

  if [[ "${DRY_RUN}" -eq 1 ]]; then
    echo "[libc-test-adapter] dry-run command: ${case_command}"

    continue
  fi

  if [[ "${uses_runtest_prefix}" -eq 1 ]] && [[ "${runtest_validated}" -eq 0 ]]; then
    if ! validate_runtest_command "${RUNTEST_COMMAND}"; then
      exit 2
    fi

    runtest_validated=1
  fi

  if ! (
    cd "${LIBC_TEST_ROOT}" && run_step 120 bash -lc "${case_command}"
  ); then
    failed_cases=$((failed_cases + 1))
    echo "[libc-test-adapter] failed smoke case ${case_id}" >&2
  fi
done < "${MANIFEST_PATH}"

if [[ "${total_cases}" -eq 0 ]]; then
  echo "[libc-test-adapter] smoke manifest has no runnable cases: ${MANIFEST_PATH}" >&2

  exit 2
fi

echo "[libc-test-adapter] smoke summary: total=${total_cases} failed=${failed_cases}"

if [[ "${DRY_RUN}" -eq 1 ]]; then
  exit 0
fi

if [[ "${failed_cases}" -ne 0 ]]; then
  exit 1
fi
