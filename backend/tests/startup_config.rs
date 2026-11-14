//! Boots the real `ois-backend` binary to prove `run()` checks `TRUSTED_PROXY_HOPS` before it builds
//! `AppState` (#923). A unit test cannot reach `run()`, and `set_var` would race every other test, so
//! each case runs the binary with its own environment.
//!
//! `DATABASE_URL` is blank in every case, so a boot that gets past the check stops at the pool's URL
//! parse with "relative URL without a base" before it connects anywhere or starts a poller. The
//! working directory is an empty temp dir, so `dotenvy` finds no `.env`.

use std::process::Command;

/// Runs the binary with `vars` and returns its exit success and its combined output, ANSI stripped.
fn boot(vars: &[(&str, &str)]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("ois-startup-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create an empty working directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ois-backend"));
    command
        .current_dir(&dir)
        .env_clear()
        .env("DATABASE_URL", "")
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .env("RUST_BACKTRACE", "0");
    for (name, value) in vars {
        command.env(name, value);
    }
    let output = command.output().expect("run ois-backend");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), strip_ansi(&text))
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Logged by `AppState::from_env`, so its absence means boot stopped before state was built.
const STATE_STARTED: &str = "nav database loaded";

#[test]
fn a_zero_or_malformed_hop_count_stops_boot_before_state_is_built() {
    for (raw, message) in [
        ("0", "TRUSTED_PROXY_HOPS must be at least 1, got 0"),
        (
            "1x",
            "TRUSTED_PROXY_HOPS must be a whole number of proxies, got \"1x\"",
        ),
        (
            "2/",
            "TRUSTED_PROXY_HOPS must be a whole number of proxies, got \"2/\"",
        ),
    ] {
        let (ok, output) = boot(&[("TRUSTED_PROXY_HOPS", raw)]);
        assert!(!ok, "{raw:?} booted: {output}");
        assert!(output.contains(message), "{raw:?}: {output}");
        assert!(!output.contains(STATE_STARTED), "{raw:?}: {output}");
    }
}

/// Positive control for the test above: a valid, blank or absent value passes the check and boot
/// goes on to build state, then stops at the blank `DATABASE_URL`.
#[test]
fn a_valid_blank_or_absent_hop_count_passes_the_check() {
    for vars in [
        &[("TRUSTED_PROXY_HOPS", "1")][..],
        &[("TRUSTED_PROXY_HOPS", "")][..],
        &[][..],
    ] {
        let (ok, output) = boot(vars);
        assert!(!ok, "{vars:?} booted with a blank DATABASE_URL: {output}");
        assert!(output.contains(STATE_STARTED), "{vars:?}: {output}");
        assert!(
            output.contains("relative URL without a base"),
            "{vars:?}: {output}"
        );
        assert!(!output.contains("TRUSTED_PROXY_HOPS"), "{vars:?}: {output}");
    }
}

/// `AppState::from_env` reads the pool size through `config::database_max_connections`, which logs
/// a bad value once and keeps the default.
#[test]
fn a_zero_pool_size_logs_one_error_naming_the_variable() {
    let (_, output) = boot(&[("DATABASE_MAX_CONNECTIONS", "0")]);
    let errors: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(" ERROR "))
        .collect();
    assert_eq!(errors.len(), 1, "{output}");
    assert!(
        errors[0].contains("DATABASE_MAX_CONNECTIONS must be a whole number above 0, got \"0\""),
        "{output}"
    );
    let (_, unset) = boot(&[]);
    assert!(!unset.contains(" ERROR "), "{unset}");
}
