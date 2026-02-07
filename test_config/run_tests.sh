#!/usr/bin/env bash
#
# Integration test runner for rotomate
#
# Usage:
#   ./test_config/run_tests.sh           # run all tests
#   ./test_config/run_tests.sh 01 03 05  # run specific tests
#
# Prerequisites:
#   - rot binary built (just build debug musl)
#   - SSH to localhost (127.0.0.1) with key-based auth (tests 02-07, 10)
#
set -euo pipefail

# --- Configuration ---
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROT="$PROJECT_DIR/target/x86_64-unknown-linux-musl/debug/rot"

# Colors
RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
YELLOW=$'\033[0;33m'
CYAN=$'\033[0;36m'
RESET=$'\033[0m'

# Counters
PASS=0
FAIL=0
SKIP=0
ERRORS=""

# --- Helpers ---

strip_ansi() {
    sed 's/\x1b\[[0-9;]*m//g'
}

assert_contains() {
    local label="$1"
    local output="$2"
    local pattern="$3"
    if echo "$output" | grep -qF "$pattern"; then
        return 0
    else
        ERRORS+="    ${RED}FAIL${RESET}: $label — expected to find: \"$pattern\"\n"
        return 1
    fi
}

assert_not_contains() {
    local label="$1"
    local output="$2"
    local pattern="$3"
    if echo "$output" | grep -qF "$pattern"; then
        ERRORS+="    ${RED}FAIL${RESET}: $label — expected NOT to find: \"$pattern\"\n"
        return 1
    else
        return 0
    fi
}

assert_exit_code() {
    local label="$1"
    local actual="$2"
    local expected="$3"
    if [ "$actual" -eq "$expected" ]; then
        return 0
    else
        ERRORS+="    ${RED}FAIL${RESET}: $label — expected exit code $expected, got $actual\n"
        return 1
    fi
}

run_rot() {
    "$ROT" "$@" 2>&1 | strip_ansi
}

begin_test() {
    local name="$1"
    ERRORS=""
    echo -n "  ${CYAN}$name${RESET} ... "
}

end_test() {
    if [ -z "$ERRORS" ]; then
        echo "${GREEN}PASS${RESET}"
        ((++PASS))
    else
        echo "${RED}FAIL${RESET}"
        echo -e "$ERRORS"
        ((++FAIL))
    fi
}

skip_test() {
    local reason="$1"
    echo "${YELLOW}SKIP${RESET} ($reason)"
    ((++SKIP))
}

# --- Tests ---

test_1000() {
    begin_test "1000 invalid_colon_space — error message for unquoted ': '"

    local out
    out=$(run_rot "$SCRIPT_DIR/1000_invalid_colon_space.yaml" --check) || true

    assert_contains "line number" "$out" "1000_invalid_colon_space.yaml:14"
    assert_contains "must be quoted" "$out" "must be quoted"
    assert_contains "suggested fix" "$out" "Suggested fix"

    end_test
}

test_1001() {
    begin_test "1001 invalid_upload_path — upload to non-existent directory"

    local out
    out=$(run_rot "$SCRIPT_DIR/1001_invalid_upload_path.yaml" -v) || true

    assert_contains "upload failed" "$out" "upload failed"
    assert_contains "no such file" "$out" "No such file"
    assert_contains "task failed" "$out" "FAILED"

    end_test
}

test_01() {
    begin_test "01 local_commands — vars, list, multi-line script"

    local out
    out=$(run_rot "$SCRIPT_DIR/01_local_commands.yaml" -v)

    assert_contains "var expansion (greeting)" "$out" "Hello from rotomate"
    assert_contains "var expansion (work_dir)" "$out" "Work dir is /tmp/rot-test-01"
    assert_contains "multi-line iteration 1" "$out" "Iteration 1"
    assert_contains "multi-line iteration 3" "$out" "Iteration 3"
    assert_contains "multi-line script done" "$out" "Done"

    end_test
}

test_02() {
    begin_test "02 remote_commands — defaults, host.*, builtin.*"

    local out
    out=$(run_rot "$SCRIPT_DIR/02_remote_commands.yaml" -v)

    assert_contains "host.name" "$out" "host.name       = localhost"
    assert_contains "host.hostname" "$out" "host.hostname   = 127.0.0.1"
    assert_contains "host.username" "$out" "host.username   = mm"
    assert_contains "builtin.host" "$out" "builtin.host    = localhost"
    assert_contains "builtin.task_group" "$out" "builtin.task_group = Remote Command Basics"

    end_test
}

test_03() {
    begin_test "03 file_transfers — upload, download, delete"

    local out
    out=$(run_rot "$SCRIPT_DIR/03_file_transfers.yaml" -v)

    assert_contains "file created" "$out" "rotomate upload test content"
    assert_contains "upload verified" "$out" "Verify upload"
    assert_contains "download verified" "$out" "Verify download"
    assert_contains "files match" "$out" "Files match!"
    assert_contains "cleanup" "$out" "Cleanup complete"

    end_test
}

test_04() {
    begin_test "04 groups_and_hosts — groups, inheritance, depends"

    local out
    out=$(run_rot "$SCRIPT_DIR/04_groups_and_hosts.yaml" -v)

    # Setup phase runs on all_servers (web1, web2, db1)
    assert_contains "setup web1" "$out" "Setup complete on web1"
    assert_contains "setup web2" "$out" "Setup complete on web2"
    assert_contains "setup db1" "$out" "Setup complete on db1"

    # Deploy phase runs on webservers only (web1, web2)
    assert_contains "deploy web1" "$out" "Deployed to web1"
    assert_contains "deploy web2" "$out" "Deployed to web2"
    assert_not_contains "deploy db1 (should not)" "$out" "Deployed to db1"

    # Verify phase runs on databases only (db1)
    assert_contains "verify db1" "$out" "Verified on db1"
    assert_not_contains "verify web1 (should not)" "$out" "Verified on web1"
    assert_not_contains "verify web2 (should not)" "$out" "Verified on web2"

    end_test
}

test_05() {
    begin_test "05 error_handling — stop_on_error true vs false"

    local out
    out=$(run_rot "$SCRIPT_DIR/05_error_handling.yaml" -v) || true

    assert_contains "strict before" "$out" "STRICT before failure"
    assert_not_contains "strict after (stopped)" "$out" "STRICT after failure"
    assert_contains "lenient before" "$out" "LENIENT before failure"
    assert_contains "lenient after (continued)" "$out" "LENIENT after failure"

    end_test
}

test_06() {
    begin_test "06 output_modes — verbose, capture, default (with -v)"

    local out
    out=$(run_rot "$SCRIPT_DIR/06_output_modes.yaml" -v)

    # With -v flag, all output should be visible
    assert_contains "verbose line" "$out" "Verbose line 1"
    assert_contains "captured line" "$out" "Captured line 1"
    assert_contains "default line" "$out" "Default line 1"

    end_test
}

test_06_capture() {
    begin_test "06 output_modes — capture with -o std"

    local out
    out=$(run_rot "$SCRIPT_DIR/06_output_modes.yaml" -o std)

    # With -o std, captured and verbose output should appear
    assert_contains "verbose output" "$out" "Verbose line 1"
    assert_contains "captured output" "$out" "Captured line 1"

    end_test
}

test_07() {
    begin_test "07 imports — config merging from multiple files"

    local out
    out=$(run_rot "$SCRIPT_DIR/07_imports/main.yaml" -v)

    assert_contains "imported task + var (server_a)" "$out" "Hello from test-app on server_a"
    assert_contains "imported task + var (server_b)" "$out" "Hello from test-app on server_b"
    assert_contains "var from imported file" "$out" "App name from imported vars: test-app"

    end_test
}

test_08_full() {
    begin_test "08 campaigns — full (auto-selected, all 3 phases)"

    local out
    out=$(run_rot "$SCRIPT_DIR/08_campaigns.yaml" -v)

    assert_contains "setup runs" "$out" "=== SETUP running ==="
    assert_contains "deploy runs" "$out" "=== DEPLOY running ==="
    assert_contains "verify runs" "$out" "=== VERIFY running ==="

    end_test
}

test_08_deploy_only() {
    begin_test "08 campaigns — deploy_only (auto-includes setup dep)"

    local out
    out=$(run_rot "$SCRIPT_DIR/08_campaigns.yaml" -v -c deploy_only)

    assert_contains "setup auto-included" "$out" "=== SETUP running ==="
    assert_contains "deploy runs" "$out" "=== DEPLOY running ==="
    assert_not_contains "verify excluded" "$out" "=== VERIFY running ==="

    end_test
}

test_08_lenient() {
    begin_test "08 campaigns — deploy_only --lenient-campaign"

    local out
    out=$(run_rot "$SCRIPT_DIR/08_campaigns.yaml" -v -c deploy_only --lenient-campaign) || true

    assert_contains "deploy runs" "$out" "=== DEPLOY running ==="
    assert_not_contains "setup excluded" "$out" "=== SETUP running ==="
    assert_not_contains "verify excluded" "$out" "=== VERIFY running ==="

    end_test
}

test_09_check() {
    begin_test "09 check_and_list — --check validates config"

    local out
    local rc=0
    out=$(run_rot "$SCRIPT_DIR/09_check_and_list.yaml" --check) || rc=$?

    assert_exit_code "--check exit code" "$rc" 0

    end_test
}

test_09_list() {
    begin_test "09 check_and_list — --list"

    local out
    out=$(run_rot "$SCRIPT_DIR/09_check_and_list.yaml" --list)

    assert_contains "host alpha" "$out" "alpha"
    assert_contains "host bravo" "$out" "bravo"
    assert_contains "host charlie" "$out" "charlie"
    assert_contains "task ping" "$out" "ping"
    assert_contains "task deploy" "$out" "deploy"
    assert_contains "task rollback" "$out" "rollback"
    assert_contains "group infra_check" "$out" "infra_check"
    assert_contains "group app_deploy" "$out" "app_deploy"
    assert_contains "group app_rollback" "$out" "app_rollback"
    assert_contains "campaign deploy_pipeline" "$out" "deploy_pipeline"
    assert_contains "campaign rollback_pipeline" "$out" "rollback_pipeline"
    assert_contains "campaign full" "$out" "full"

    end_test
}

test_11() {
    begin_test "11 steps — ordered operations in a single task"

    local out
    out=$(run_rot "$SCRIPT_DIR/11_steps.yaml" -v)

    assert_contains "step1 remote_command" "$out" "STEP1 remote_command on localhost"
    assert_contains "step3 verify upload" "$out" "STEP3 verify upload"
    assert_contains "upload content" "$out" "steps-test-content"
    assert_contains "step4 script style" "$out" "STEP4 script style on localhost"
    assert_contains "step4 wc output" "$out" "line(s)"
    assert_contains "step5 verify download" "$out" "STEP5 verify download"
    assert_contains "files match" "$out" "Files match!"
    assert_contains "step6 local script" "$out" "STEP6 local script style"
    assert_contains "step6 byte count" "$out" "bytes"
    assert_contains "step7 cleanup" "$out" "STEP7 remote cleanup done"
    assert_contains "local cleanup" "$out" "Local cleanup complete"

    end_test
}

test_1002() {
    begin_test "1002 timeout — sleep beyond default timeout"

    local out
    out=$(run_rot "$SCRIPT_DIR/1002_inactivity_timeout.yaml" -v) || true

    assert_contains "before sleep ran" "$out" "before sleep"
    assert_not_contains "after sleep (timed out)" "$out" "after sleep"
    assert_contains "timeout error" "$out" "inactivity timeout"
    assert_contains "task failed" "$out" "FAILED"

    end_test
}

# --- Main ---

echo ""
echo "=== rotomate integration tests ==="
echo ""

# Check binary exists
if [ ! -x "$ROT" ]; then
    echo "${RED}ERROR:${RESET} rot binary not found at $ROT"
    echo "       Run: just build release musl"
    exit 1
fi

# Determine which tests to run
REQUESTED=("$@")

should_run() {
    local prefix="$1"
    if [ ${#REQUESTED[@]} -eq 0 ]; then
        return 0  # no filter — run all
    fi
    for r in "${REQUESTED[@]}"; do
        if [[ "$prefix" == "$r"* ]]; then
            return 0
        fi
    done
    return 1
}

# Error message tests (no SSH required)
if should_run "1000"; then test_1000; fi

# Local-only tests (no SSH required)
if should_run "01"; then test_01; fi
if should_run "08"; then
    test_08_full
    test_08_deploy_only
    test_08_lenient
fi
if should_run "09"; then
    test_09_check
    test_09_list
fi

# SSH-required tests
if should_run "02"; then test_02; fi
if should_run "03"; then test_03; fi
if should_run "04"; then test_04; fi
if should_run "05"; then test_05; fi
if should_run "06"; then
    test_06
    test_06_capture
fi
if should_run "07"; then test_07; fi

if should_run "11"; then test_11; fi
if should_run "1001"; then test_1001; fi
if should_run "1002"; then test_1002; fi

# Test 10 (sudo) is always skipped in automated runs
if should_run "10"; then
    begin_test "10 sudo — become_root (requires interactive password)"
    skip_test "requires --root interactive prompt"
fi

# --- Summary ---
echo ""
TOTAL=$((PASS + FAIL + SKIP))
echo "=== Results: ${GREEN}$PASS passed${RESET}, ${RED}$FAIL failed${RESET}, ${YELLOW}$SKIP skipped${RESET} (${TOTAL} total) ==="
echo ""

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
