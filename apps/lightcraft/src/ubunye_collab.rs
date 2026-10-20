//! Ubunye: starts ThabangVision collaboration (`kopanod`, the Kopano service) for this computer.
//!
//! The installers put `kopanod` next to the app (`ubunye/packaging/`). On every launch, off the
//! UI thread, this:
//! 1. finds that `kopanod` (none, as in a developer build: nothing happens);
//! 2. makes the device key once (`kopanod init`; it goes into the OS key store);
//! 3. turns start-at-sign-in on (`kopanod autostart on`) when it is not already set to this
//!    `kopanod`, so a moved or upgraded install fixes itself;
//! 4. starts `kopanod run` when nothing answers on its port yet.
//!
//! All of it is silent: no console window, no dialog. A failure is only logged; the
//! Share & Collaborate window then says collaboration is not running. Uninstalling turns
//! start-at-sign-in off again (the MSI and the Linux packages; see ubunye/packaging/README.md).
//!
//! `UBUNYE_COLLAB=0` turns all of this off, `UBUNYE_COLLAB_AUTOSTART=0` only step 3.
//! `KOPANO_BIN` points at another `kopanod`; `KOPANO_HOME` / `KOPANO_API_PORT` as in `kopanod`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lightcraft_ui_egui::ubunye::kopano::Config;

/// The service's file name next to the app.
const EXE: &str = if cfg!(windows) { "kopanod.exe" } else { "kopanod" };
/// What the Run value is called on Windows (kopanod's own `RUN_VALUE`).
const RUN_VALUE: &str = "ThabangVision collaboration";
/// The log kopanod writes when this app starts it (kept short: trimmed past this size).
const LOG_MAX: u64 = 4 << 20;

/// Start the check on its own thread; returns at once.
pub fn start_in_background() {
    if off("UBUNYE_COLLAB") {
        return;
    }
    let spawned = std::thread::Builder::new().name("collab-start".into()).spawn(|| {
        if let Err(e) = ensure_running(&Config::from_env()) {
            log::warn!("collaboration service: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("collaboration service: could not start the check: {e}");
    }
}

fn off(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim(), "0" | "off" | "false" | "no"))
}

/// The `kopanod` this app ships with: `KOPANO_BIN`, else next to this program.
pub fn find_kopanod() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KOPANO_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(EXE);
    p.is_file().then_some(p)
}

/// Is something answering on the service's port?
pub fn answers(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Where start-at-sign-in can point: a path that stays put. An AppImage runs from a temporary
/// mount, so its `kopanod` is copied into the service folder first; a Mac app still on its disk
/// image or not yet moved out of quarantine (App Translocation) gets no autostart until it is.
pub fn lasting_copy(exe: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    let s = exe.to_string_lossy();
    if cfg!(target_os = "macos") && (s.contains("/AppTranslocation/") || s.starts_with("/Volumes/")) {
        return Ok(None);
    }
    // Windows: only the installed app (the MSI puts it under Program Files). A portable copy can
    // be moved or deleted, which would leave a broken startup entry behind.
    if cfg!(windows) && !installed_on_windows(exe) {
        return Ok(None);
    }
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some() {
        let dir = home.join("bin");
        let to = dir.join(EXE);
        let same = std::fs::read(&to).ok().zip(std::fs::read(exe).ok()).is_some_and(|(a, b)| a == b);
        if !same {
            std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
            let tmp = dir.join(format!("{EXE}.new"));
            std::fs::copy(exe, &tmp).map_err(|e| format!("could not copy {}: {e}", exe.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
            }
            std::fs::rename(&tmp, &to).map_err(|e| format!("could not place {}: {e}", to.display()))?;
        }
        return Ok(Some(to));
    }
    Ok(Some(exe.to_path_buf()))
}

/// Is `exe` inside a Program Files folder (where the MSI installs it)?
fn installed_on_windows(exe: &Path) -> bool {
    let exe = exe.to_string_lossy().to_lowercase();
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .any(|dir| exe.starts_with(&format!("{}\\", dir.trim_end_matches('\\').to_lowercase())))
}

/// The start-at-sign-in entry kopanod writes, as text (`None`: there is none).
fn autostart_entry() -> Option<String> {
    if cfg!(windows) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let out = hidden(Command::new("reg").args(["query", key, "/v", RUN_VALUE])).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let f = if cfg!(target_os = "macos") {
            home.join("Library/LaunchAgents/org.ubunye.kopano.plist")
        } else {
            home.join(".config/systemd/user/kopano.service")
        };
        std::fs::read_to_string(f).ok()
    }
}

/// Does the entry start this `exe` with this home and port?
pub fn entry_points_at(entry: &str, exe: &Path, home: &Path, port: u16) -> bool {
    let has = |s: &str| {
        if cfg!(windows) { entry.to_lowercase().contains(&s.to_lowercase()) } else { entry.contains(s) }
    };
    has(&exe.to_string_lossy()) && has(&home.to_string_lossy()) && has(&port.to_string())
}

/// No console window for a child of this GUI app (Windows).
fn hidden(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        c.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    c.stdin(Stdio::null())
}

/// Run `kopanod <args>` and wait (at most `limit`), for init and autostart.
fn run_once(exe: &Path, args: &[&str], cfg: &(PathBuf, u16), limit: Duration) -> Result<String, String> {
    let port = cfg.1.to_string();
    let mut child = hidden(Command::new(exe).args(args).arg("--home").arg(&cfg.0).args(["--api-port", &port]))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("kopanod {} took too long", args.join(" ")));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("kopanod {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The whole check (see the module docs).
pub fn ensure_running(config: &Config) -> Result<(), String> {
    let Some(exe) = find_kopanod() else {
        log::info!("collaboration service: no kopanod next to this app (a developer build); not starting it");
        return Ok(());
    };
    let home = config.home.clone().ok_or("the collaboration folder is unknown here (set KOPANO_HOME)")?;
    let port = config.port;
    std::fs::create_dir_all(&home).map_err(|e| format!("could not make {}: {e}", home.display()))?;
    let running = answers(port);
    // start-at-sign-in only for a kopanod that stays where it is (see lasting_copy)
    let (exe, lasting) = match lasting_copy(&exe, &home) {
        Ok(Some(p)) => (p, true),
        Ok(None) => (exe, false),
        Err(e) => {
            log::warn!("collaboration service: {e}");
            (exe, false)
        }
    };
    let cfg = (home.clone(), port);
    if !running {
        let id = run_once(&exe, &["init"], &cfg, Duration::from_secs(30))?;
        log::info!("collaboration service: this computer is {}", id.get(..8).unwrap_or(&id));
    }
    let autostart = lasting && !off("UBUNYE_COLLAB_AUTOSTART");
    if autostart && !autostart_entry().is_some_and(|e| entry_points_at(&e, &exe, &home, port)) {
        match run_once(&exe, &["autostart", "on"], &cfg, Duration::from_secs(30)) {
            Ok(_) => log::info!("collaboration service: starts when you sign in to this computer"),
            Err(e) => log::warn!("collaboration service: start at sign-in not set: {e}"),
        }
    }
    // macOS and Linux start it with the autostart entry; give that a moment
    let start = Instant::now();
    while !answers(port) {
        if start.elapsed() > Duration::from_secs(if autostart && !cfg!(windows) { 3 } else { 0 }) {
            return spawn_run(&exe, &home, port);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Start `kopanod run` on its own; it keeps running after this app quits.
fn spawn_run(exe: &Path, home: &Path, port: u16) -> Result<(), String> {
    let log_path = home.join("kopanod.log");
    if std::fs::metadata(&log_path).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::rename(&log_path, home.join("kopanod.old.log"));
    }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    hidden(Command::new(exe).arg("run").arg("--home").arg(home).args(["--api-port", &port.to_string()]))
        .stdout(log)
        .stderr(log2)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    log::info!("collaboration service: started {}", exe.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_must_name_this_exe_home_and_port() {
        let exe = PathBuf::from("/Apps/Light/kopanod");
        let home = PathBuf::from("/h/kopano");
        let e = "ExecStart=\"/Apps/Light/kopanod\" run --home \"/h/kopano\" --api-port 47100";
        assert!(entry_points_at(e, &exe, &home, 47100));
        assert!(!entry_points_at(e, &exe, &home, 18571));
        assert!(!entry_points_at(e, &PathBuf::from("/Other/kopanod"), &home, 47100));
        assert!(!entry_points_at(e, &exe, &PathBuf::from("/x"), 47100));
    }

    #[test]
    fn switches_read_plain_words() {
        assert!(!off("UBUNYE_COLLAB_TEST_UNSET_VARIABLE"));
    }

    #[test]
    fn a_missing_kopanod_is_not_an_error() {
        let cfg = Config { port: 1, home: Some(std::env::temp_dir().join("ubunye-collab-test-none")) };
        // the test binary has no kopanod next to it
        if find_kopanod().is_none() {
            assert_eq!(ensure_running(&cfg), Ok(()));
        }
    }
}
