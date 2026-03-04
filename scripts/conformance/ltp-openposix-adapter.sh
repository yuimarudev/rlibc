#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ltp-openposix-adapter.sh --suite <ltp|open_posix_testsuite> --suite-root <path> [options] [-- <command> [args...]]

Options:
  --suite <name>            Suite identifier (`ltp` or `open_posix_testsuite`).
  --suite-root <path>       Root directory of the external suite checkout.
  --results-file <path>     Explicit result file path to parse.
  --timeout-seconds <sec>   Timeout for optional command execution (default: 120).
  --help                    Show this help.

Result file format:
  One case result per line:
    <STATUS> <CASE_ID...>

  Supported statuses:
    PASS, TPASS
    FAIL, TFAIL, BROK, TBROK
    SKIP, TSKIP
    XFAIL
    XPASS
    TIMEOUT, TTIME

Output:
  Deterministic key-value summary on stdout:
    suite=...
    suite_root=...
    results_file=...
    pass=...
    fail=...
    skip=...
    xfail=...
    xpass=...
    timeout=...
    error=...
    failed_cases=case-a,case-b
EOF
}

die() {
  echo "[i061-adapter] error: $*" >&2
  exit 2
}

resolve_timeout_bin() {
  if command -v timeout >/dev/null 2>&1; then
    echo "timeout"

    return
  fi

  if command -v gtimeout >/dev/null 2>&1; then
    echo "gtimeout"

    return
  fi

  die "timeout command not found (expected 'timeout' or 'gtimeout')"
}

trim_spaces() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

canonicalize_dir() {
  local dir_path="$1"

  (
    cd "${dir_path}" >/dev/null 2>&1 || exit 1
    pwd -P
  )
}

canonicalize_file() {
  local file_path="$1"
  local file_dir
  local file_base
  local canonical_dir

  file_dir="$(dirname -- "${file_path}")"
  file_base="$(basename -- "${file_path}")"
  canonical_dir="$(canonicalize_dir "${file_dir}")" || return 1
  printf '%s/%s' "${canonical_dir}" "${file_base}"
}

normalize_absolute_path_lexically() {
  local absolute_path="$1"
  local remainder
  local segment
  local index
  local normalized_path
  local -a normalized_segments=()

  if [[ "${absolute_path}" != /* ]]; then
    return 1
  fi

  remainder="${absolute_path#/}"
  while [[ -n "${remainder}" ]]; do
    if [[ "${remainder}" == */* ]]; then
      segment="${remainder%%/*}"
      remainder="${remainder#*/}"
    else
      segment="${remainder}"
      remainder=""
    fi

    if [[ -z "${segment}" || "${segment}" == "." ]]; then
      continue
    fi

    if [[ "${segment}" == ".." ]]; then
      if ((${#normalized_segments[@]} > 0)); then
        unset "normalized_segments[${#normalized_segments[@]}-1]"
      fi
      continue
    fi

    normalized_segments+=("${segment}")
  done

  if ((${#normalized_segments[@]} == 0)); then
    printf '/'

    return 0
  fi

  normalized_path="/${normalized_segments[0]}"
  for ((index = 1; index < ${#normalized_segments[@]}; index++)); do
    normalized_path+="/${normalized_segments[${index}]}"
  done

  printf '%s' "${normalized_path}"
}

path_contains_dot_segments() {
  local path="$1"
  local remainder
  local segment

  remainder="${path}"
  while [[ "${remainder}" == /* ]]; do
    remainder="${remainder#/}"
  done
  while [[ -n "${remainder}" ]]; do
    if [[ "${remainder}" == */* ]]; then
      segment="${remainder%%/*}"
      remainder="${remainder#*/}"
    else
      segment="${remainder}"
      remainder=""
    fi

    if [[ -z "${segment}" ]]; then
      continue
    fi

    if [[ "${segment}" == "." || "${segment}" == ".." ]]; then
      return 0
    fi
  done

  return 1
}

validate_results_file_inside_suite_root() {
  local normalized_results_file="$1"

  case "${normalized_results_file}" in
    "${suite_root}" | "${suite_root}"/*)
      ;;
    *)
      die "results file must be inside suite root: ${normalized_results_file}"
      ;;
  esac
}

find_nearest_existing_ancestor_dir() {
  local path="$1"
  local candidate

  candidate="${path}"
  if [[ ! -d "${candidate}" ]]; then
    candidate="$(dirname -- "${candidate}")"
  fi

  while [[ ! -d "${candidate}" && "${candidate}" != "/" ]]; do
    candidate="$(dirname -- "${candidate}")"
  done

  [[ -d "${candidate}" ]] || return 1
  printf '%s' "${candidate}"
}

resolve_and_validate_results_file() {
  local candidate="$1"
  local candidate_for_dot_segment_check
  local normalized_candidate
  local candidate_ancestor
  local candidate_dir
  local canonical_candidate

  candidate_for_dot_segment_check="${candidate}"
  if [[ "${candidate_for_dot_segment_check}" != /* ]]; then
    candidate_for_dot_segment_check="/${candidate_for_dot_segment_check}"
  fi
  if path_contains_dot_segments "${candidate_for_dot_segment_check}"; then
    die "results file path must not contain dot segments: ${candidate}"
  fi

  if [[ "${candidate}" != /* ]]; then
    candidate="${suite_root}/${candidate}"
  fi

  if [[ "${candidate}" != "/" && "${candidate}" == */ ]]; then
    die "results file path must not end with trailing slash: ${candidate}"
  fi

  normalized_candidate="$(normalize_absolute_path_lexically "${candidate}")" || die "failed to normalize results file path: ${candidate}"
  validate_results_file_inside_suite_root "${normalized_candidate}"

  candidate_ancestor="$(find_nearest_existing_ancestor_dir "${normalized_candidate}")" || die "failed to find existing ancestor directory for results file: ${normalized_candidate}"
  candidate_ancestor="$(canonicalize_dir "${candidate_ancestor}")" || die "failed to canonicalize existing ancestor directory for results file: ${normalized_candidate}"
  validate_results_file_inside_suite_root "${candidate_ancestor}"

  if [[ -d "${normalized_candidate}" ]]; then
    die "results file path must reference a regular file: ${normalized_candidate}"
  fi

  candidate_dir="$(dirname -- "${normalized_candidate}")"
  if [[ -d "${candidate_dir}" ]]; then
    canonical_candidate="$(canonicalize_file "${normalized_candidate}")" || die "failed to canonicalize results file: ${normalized_candidate}"
    validate_results_file_inside_suite_root "${canonical_candidate}"

    if [[ -L "${canonical_candidate}" ]]; then
      die "results file must not be a symlink: ${canonical_candidate}"
    fi
    normalized_candidate="${canonical_candidate}"
  fi

  if [[ -e "${normalized_candidate}" && ! -f "${normalized_candidate}" ]]; then
    die "results file path must reference a regular file: ${normalized_candidate}"
  fi

  printf '%s' "${normalized_candidate}"
}

suite=""
suite_explicitly_set=0
suite_root=""
suite_root_explicitly_set=0
declare -a explicit_suite_root_values=()
results_file=""
results_file_explicitly_set=0
timeout_seconds="120"
declare -a explicit_results_file_values=()

while (($# > 0)); do
  case "$1" in
    --suite)
      (($# >= 2)) || die "--suite requires a value"
      if [[ "$2" == "--" ]]; then
        die "--suite requires a value"
      fi
      if [[ -z "$2" ]]; then
        die "--suite must not be empty"
      fi
      if [[ "$2" != "ltp" && "$2" != "open_posix_testsuite" ]]; then
        die "--suite must be one of: ltp, open_posix_testsuite"
      fi
      suite="$2"
      suite_explicitly_set=1
      shift 2
      ;;
    --suite-root)
      (($# >= 2)) || die "--suite-root requires a value"
      if [[ "$2" == "--" ]]; then
        die "--suite-root requires a value"
      fi
      if [[ -z "$2" ]]; then
        die "--suite-root must not be empty"
      fi
      if [[ ! -e "$2" ]]; then
        die "suite root does not exist: $2"
      fi
      if [[ ! -d "$2" ]]; then
        die "suite root must be a directory: $2"
      fi
      canonicalize_dir "$2" >/dev/null || die "failed to canonicalize suite root: $2"
      explicit_suite_root_values+=("$2")
      suite_root="$2"
      suite_root_explicitly_set=1
      if ((${#explicit_results_file_values[@]} > 0)); then
        suite_root_for_parse_time_results_validation="${suite_root}"
        suite_root="$(canonicalize_dir "${suite_root_for_parse_time_results_validation}")" || die "failed to canonicalize suite root: ${suite_root_for_parse_time_results_validation}"
        for explicit_results_file in "${explicit_results_file_values[@]}"; do
          resolve_and_validate_results_file "${explicit_results_file}" >/dev/null
        done
        suite_root="${suite_root_for_parse_time_results_validation}"
      fi
      shift 2
      ;;
    --results-file)
      (($# >= 2)) || die "--results-file requires a value"
      if [[ "$2" == "--" ]]; then
        die "--results-file requires a value"
      fi
      if [[ -z "$2" ]]; then
        die "--results-file must not be empty"
      fi
      if [[ "$2" != "/" && "$2" == */ ]]; then
        die "results file path must not end with trailing slash: $2"
      fi
      results_file_path_for_dot_segment_check="$2"
      if [[ "${results_file_path_for_dot_segment_check}" != /* ]]; then
        results_file_path_for_dot_segment_check="/${results_file_path_for_dot_segment_check}"
      fi
      if path_contains_dot_segments "${results_file_path_for_dot_segment_check}"; then
        die "results file path must not contain dot segments: $2"
      fi
      if [[ -n "${suite_root}" ]]; then
        suite_root_for_parse_time_results_validation="${suite_root}"
        suite_root="$(canonicalize_dir "${suite_root_for_parse_time_results_validation}")" || die "failed to canonicalize suite root: ${suite_root_for_parse_time_results_validation}"
        resolve_and_validate_results_file "$2" >/dev/null
        suite_root="${suite_root_for_parse_time_results_validation}"
      fi
      explicit_results_file_values+=("$2")
      results_file="$2"
      results_file_explicitly_set=1
      shift 2
      ;;
    --timeout-seconds)
      (($# >= 2)) || die "--timeout-seconds requires a value"
      if [[ "$2" == "--" ]]; then
        die "--timeout-seconds requires a value"
      fi
      if [[ -z "$2" ]]; then
        die "--timeout-seconds must not be empty"
      fi
      if [[ ! "$2" =~ ^[1-9][0-9]*$ ]]; then
        die "--timeout-seconds must be a positive integer"
      fi
      timeout_seconds="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

if ((suite_explicitly_set)) && [[ -z "${suite}" ]]; then
  die "--suite must not be empty"
fi

if [[ -z "${suite}" ]]; then
  die "--suite is required"
fi

if [[ "${suite}" != "ltp" && "${suite}" != "open_posix_testsuite" ]]; then
  die "--suite must be one of: ltp, open_posix_testsuite"
fi

if ((suite_root_explicitly_set)) && [[ -z "${suite_root}" ]]; then
  die "--suite-root must not be empty"
fi

if [[ -z "${suite_root}" ]]; then
  die "--suite-root is required"
fi

if [[ ! -e "${suite_root}" ]]; then
  die "suite root does not exist: ${suite_root}"
fi

if [[ ! -d "${suite_root}" ]]; then
  die "suite root must be a directory: ${suite_root}"
fi

if ((${#explicit_suite_root_values[@]} > 0)); then
  for explicit_suite_root in "${explicit_suite_root_values[@]}"; do
    canonicalize_dir "${explicit_suite_root}" >/dev/null || die "failed to canonicalize suite root: ${explicit_suite_root}"
  done
fi

if [[ -z "${timeout_seconds}" ]]; then
  die "--timeout-seconds must not be empty"
fi

if [[ ! "${timeout_seconds}" =~ ^[1-9][0-9]*$ ]]; then
  die "--timeout-seconds must be a positive integer"
fi

if ((results_file_explicitly_set)) && [[ -z "${results_file}" ]]; then
  die "--results-file must not be empty"
fi

suite_root="$(canonicalize_dir "${suite_root}")" || die "failed to canonicalize suite root: ${suite_root}"

if [[ -z "${results_file}" ]]; then
  case "${suite}" in
    ltp)
      results_file="${suite_root}/results/ltp-results.txt"
      ;;
    open_posix_testsuite)
      results_file="${suite_root}/results/open-posix-results.txt"
      ;;
  esac
fi

if ((${#explicit_results_file_values[@]} > 0)); then
  for explicit_results_file in "${explicit_results_file_values[@]}"; do
    resolve_and_validate_results_file "${explicit_results_file}" >/dev/null
  done
fi

results_file="$(resolve_and_validate_results_file "${results_file}")"

if (($# > 0)); then
  timeout_bin="$(resolve_timeout_bin)"

  set +e
  "${timeout_bin}" "${timeout_seconds}" "$@"
  command_status="$?"
  set -e

  if [[ "${command_status}" -eq 124 ]]; then
    cat <<EOF
suite=${suite}
suite_root=${suite_root}
results_file=${results_file}
pass=0
fail=0
skip=0
xfail=0
xpass=0
timeout=1
error=0
failed_cases=__adapter_timeout__:${timeout_seconds}
EOF
    exit 1
  fi

  if [[ "${command_status}" -ne 0 ]]; then
    echo "[i061-adapter] error: suite command failed with exit status ${command_status}" >&2
    cat <<EOF
suite=${suite}
suite_root=${suite_root}
results_file=${results_file}
pass=0
fail=0
skip=0
xfail=0
xpass=0
timeout=0
error=1
failed_cases=__adapter_command_failed__:${command_status}
EOF
    exit 1
  fi
fi

results_file="$(canonicalize_file "${results_file}")" || die "failed to canonicalize results file: ${results_file}"
validate_results_file_inside_suite_root "${results_file}"

if [[ -L "${results_file}" ]]; then
  die "results file must not be a symlink: ${results_file}"
fi

if [[ -e "${results_file}" && ! -f "${results_file}" ]]; then
  die "results file path must reference a regular file: ${results_file}"
fi

if [[ ! -f "${results_file}" ]]; then
  die "results file does not exist: ${results_file}"
fi

pass_count=0
fail_count=0
skip_count=0
xfail_count=0
xpass_count=0
timeout_count=0
error_count=0
line_number=0
failed_cases=()

while IFS= read -r raw_line || [[ -n "${raw_line}" ]]; do
  line_number=$((line_number + 1))
  line="$(trim_spaces "${raw_line}")"

  if [[ -z "${line}" || "${line}" == \#* ]]; then
    continue
  fi

  status="${line%%[[:space:]]*}"
  case_id="$(trim_spaces "${line#${status}}")"

  if [[ -z "${case_id}" ]]; then
    case_id="${suite}-line-${line_number}"
  fi

  case "${status}" in
    PASS|TPASS)
      pass_count=$((pass_count + 1))
      ;;
    FAIL|TFAIL|BROK|TBROK)
      fail_count=$((fail_count + 1))
      failed_cases+=("${case_id}")
      ;;
    SKIP|TSKIP)
      skip_count=$((skip_count + 1))
      ;;
    XFAIL)
      xfail_count=$((xfail_count + 1))
      ;;
    XPASS)
      xpass_count=$((xpass_count + 1))
      failed_cases+=("${case_id}")
      ;;
    TIMEOUT|TTIME)
      timeout_count=$((timeout_count + 1))
      failed_cases+=("${case_id}")
      ;;
    *)
      echo "[i061-adapter] error: unknown status '${status}' in ${results_file}:${line_number}" >&2
      error_count=$((error_count + 1))
      failed_cases+=("${case_id}")
      ;;
  esac
done < "${results_file}"

failed_cases_csv=""
for case_id in "${failed_cases[@]}"; do
  if [[ -z "${failed_cases_csv}" ]]; then
    failed_cases_csv="${case_id}"
  else
    failed_cases_csv="${failed_cases_csv},${case_id}"
  fi
done

cat <<EOF
suite=${suite}
suite_root=${suite_root}
results_file=${results_file}
pass=${pass_count}
fail=${fail_count}
skip=${skip_count}
xfail=${xfail_count}
xpass=${xpass_count}
timeout=${timeout_count}
error=${error_count}
failed_cases=${failed_cases_csv}
EOF

if ((fail_count > 0 || timeout_count > 0 || xpass_count > 0 || error_count > 0)); then
  exit 1
fi
