//! Startup config tests (#923). The boot cases run the real `ois-backend` binary to prove `run()`
//! checks `TRUSTED_PROXY_HOPS` before it builds `AppState`: a unit test cannot reach `run()`, and
//! `set_var` would race every other test, so each boot gets its own environment. The last case runs
//! in-process, capturing the log of `config::rate_limit_per_min` read from `PATH`.
//!
//! `DATABASE_URL` is blank in every boot, so a boot that gets past the check stops at the pool's URL
//! parse with "relative URL without a base" before it connects anywhere or starts a poller. Each boot
//! runs in a fresh temp dir. `dotenvy` also reads a `.env` in any parent directory, so `boot` refuses
//! to run when one exists there.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Runs the binary with `vars` and returns its exit success and its combined output, ANSI stripped.
fn boot(vars: &[(&str, &str)]) -> (bool, String) {
    static BOOTS: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ois-startup-config-{}-{}",
        std::process::id(),
        BOOTS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).expect("create a fresh working directory");
    if let Some(found) = dir
        .ancestors()
        .map(|d| d.join(".env"))
        .find(|f| Path::new(f).exists())
    {
        panic!(
            "{} would load into the child; remove it to run these tests",
            found.display()
        );
    }
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
    std::fs::remove_dir_all(&dir).expect("remove the working directory");
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
    assert!(errors[0].contains("default=20"), "{output}");
    let (_, unset) = boot(&[]);
    assert!(!unset.contains(" ERROR "), "{unset}");
}

/// `rate_limit_per_min` reads the variable it is named and logs a bad value (#923). `PATH` is set in
/// every test process and is never a number, so this needs no `set_var`.
#[test]
fn a_rate_limit_read_from_a_set_malformed_variable_logs_one_error_and_keeps_its_default() {
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(std::env::var("PATH").is_ok_and(|path| path.parse::<u32>().is_err()));
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let limit = tracing::subscriber::with_default(subscriber, || {
        ois_backend::config::rate_limit_per_min("PATH", 37)
    });
    assert_eq!(limit.get(), 37);
    let logged = String::from_utf8(capture.0.lock().expect("capture lock").clone()).unwrap();
    let lines: Vec<&str> = logged.lines().collect();
    assert_eq!(lines.len(), 1, "{logged}");
    assert!(
        lines[0].contains("ERROR") && lines[0].contains("variable=\"PATH\""),
        "{logged}"
    );
}
