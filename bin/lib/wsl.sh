#!/bin/bash
# Settings and probes for the WSL gate clone (bin/wsl-clone.sh). Sourced, never executed;
# side-effect-free on source. Tests: bin/lib/wsl-clone.test.sh.
#
# This file is the one place the WSL build env and the preflight thresholds are set.

# Below either figure a long build does not start. A cold target/ takes about 15 GB on C:
# (Ubuntu's ext4.vhdx grows there and does not shrink by itself), and one ois_backend rustc
# peaks near 8.5 GB under the 12 GB .wslconfig cap. GiB, compared in KiB.
WSL_MIN_C_FREE_GB=20
WSL_MIN_MEM_AVAILABLE_GB=9

# One crate at a time, one rustc thread. The target-specific variable is appended after
# .cargo/config.toml's list, so its -Zthreads=1 wins and the mold link flag stays; plain
# RUSTFLAGS would replace the whole list and drop mold.
WSL_CARGO_BUILD_JOBS=1
WSL_RUSTFLAGS_VAR=CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS
WSL_RUSTFLAGS=-Zthreads=1

# Where WSL mounts the Windows drives: a node or pnpm under it is Windows's copy (interop).
WSL_WINDOWS_MOUNT="${WSL_WINDOWS_MOUNT:-/mnt}"

# wsl_build_env_lines: the KEY=VALUE lines bin/wsl-clone.sh writes into the clone's .env,
# which bin/run-tests.sh (`just test`) and bin/pre-push load with `set -a`.
wsl_build_env_lines() {
    printf 'CARGO_BUILD_JOBS=%s\n' "$WSL_CARGO_BUILD_JOBS"
    printf '%s=%s\n' "$WSL_RUSTFLAGS_VAR" "$WSL_RUSTFLAGS"
}

# wsl_clone_name BRANCH: the clone's directory name, `/` written as `-`.
wsl_clone_name() {
    printf '%s\n' "${1//\//-}"
}

# wsl_db_name NAME: the clone's throwaway database. Lower-cased, every character outside
# [a-z0-9_] folded to `_`, cut to Postgres's 63-byte identifier limit.
wsl_db_name() {
    local id
    id="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | LC_ALL=C sed 's/[^a-z0-9_]/_/g')"
    [[ -n "$id" ]] || return 1
    printf 'ois_wsl_%s\n' "$id" | cut -c1-63
}

# wsl_mount_path PATH: a worktree path from `git worktree list` as WSL can open it. Linux git
# prints a worktree Windows git registered as <common dir>/worktrees/<id>/C:/..., reading its
# drive path as relative; when PATH is not a directory, its drive path is read through /mnt.
wsl_mount_path() {
    if [[ -z "$1" || -d "$1" ]]; then
        printf '%s\n' "$1"
        return 0
    fi
    printf '%s\n' "$1" | sed -E 's#\\#/#g; s#^(.*/)?([A-Za-z]):/#/mnt/\L\2/#'
}

# wsl_c_free_kb: free KiB on Windows C:, through its /mnt/c mount. WSL's own `df /` reports
# the virtual disk, which has room long after C: is full.
wsl_c_free_kb() {
    df -Pk /mnt/c 2>/dev/null | awk 'NR == 2 && $4 ~ /^[0-9]+$/ { print $4 }'
}

# wsl_mem_available_kb: MemAvailable inside the WSL VM, in KiB.
wsl_mem_available_kb() {
    awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ { print $2 }' "${WSL_MEMINFO:-/proc/meminfo}" 2>/dev/null
}

# wsl_preflight: prints both figures against their thresholds. Returns 1 when either is
# under its threshold or cannot be read: an unreadable figure is not a pass.
wsl_preflight() {
    local free mem rc=0
    local min_free=$((WSL_MIN_C_FREE_GB * 1024 * 1024)) min_mem=$((WSL_MIN_MEM_AVAILABLE_GB * 1024 * 1024))
    free="$(wsl_c_free_kb)"
    mem="$(wsl_mem_available_kb)"
    if [[ -z "$free" ]]; then
        echo "preflight: cannot read C: free space (df -Pk /mnt/c)"; rc=1
    elif (( free < min_free )); then
        echo "preflight: C: free $((free / 1024 / 1024)) GiB ($free KiB) is under ${WSL_MIN_C_FREE_GB} GiB"; rc=1
    else
        echo "preflight: C: free $((free / 1024 / 1024)) GiB (minimum ${WSL_MIN_C_FREE_GB} GiB)"
    fi
    if [[ -z "$mem" ]]; then
        echo "preflight: cannot read WSL MemAvailable (${WSL_MEMINFO:-/proc/meminfo})"; rc=1
    elif (( mem < min_mem )); then
        echo "preflight: WSL MemAvailable $((mem / 1024 / 1024)) GiB ($mem KiB) is under ${WSL_MIN_MEM_AVAILABLE_GB} GiB"; rc=1
    else
        echo "preflight: WSL MemAvailable $((mem / 1024 / 1024)) GiB (minimum ${WSL_MIN_MEM_AVAILABLE_GB} GiB)"
    fi
    return "$rc"
}
