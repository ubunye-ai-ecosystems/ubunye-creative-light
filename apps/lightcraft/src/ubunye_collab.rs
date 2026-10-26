//! Ubunye: starts ThabangVision collaboration (`kopanod`, the Kopano service) for this computer.
//!
//! Light, Photo and Film all carry `kopanod`, but a computer runs one, whatever apps are
//! installed (`ubunye/packaging/README.md`):
//! - Windows: the MSI puts the shared one in `Program Files\ThabangVision\Collaboration` with a
//!   machine-wide start-at-sign-in value; Windows Installer counts the apps that use it.
//! - Linux `.deb` / `.rpm`: `/usr/bin/kopanod` and a packaged user unit link to the newest copy
//!   installed (update-alternatives).
//! - macOS: the LaunchAgent points at the `kopanod` of the newest app; an app only repoints it
//!   when that file is gone or older than its own.
//!
//! On every launch, off the UI thread, this:
//! 1. finds the shared `kopanod`, else the one next to the app (a developer build has none:
//!    nothing happens);
//! 2. makes the device key once (`kopanod init`; it goes into the OS key store);
//! 3. leaves start-at-sign-in to the installer where there is one (and clears a per-user entry
//!    older builds made); otherwise turns it on (`kopanod autostart on`) unless it already
//!    starts a good `kopanod` (macOS: one at least as new as this app's);
//! 4. starts `kopanod` when nothing answers on its port yet.
//!
//! All of it is silent: no console window, no dialog. A failure is only logged; the
//! Share & Collaborate window then says collaboration is not running.
//!
//! `UBUNYE_COLLAB=0` turns all of this off, `UBUNYE_COLLAB_AUTOSTART=0` only step 3.
//! `KOPANO_BIN` points at another `kopanod`; `KOPANO_HOME` / `KOPANO_API_PORT` as in `kopanod`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lightcraft_ui_egui::ubunye::kopano::Config;

/// The service's file name next to the app.
const EXE: &str = if cfg!(windows) { "kopanod.exe" } else { "kopanod" };
/// What the Run value is called on Windows (kopanod's own `RUN_VALUE`, and the MSI's).
const RUN_VALUE: &str = "ThabangVision collaboration";
/// The shared `kopanod` the MSI installs, under Program Files (`ubunye/packaging/windows/collab.wxs`).
const WINDOWS_SHARED: &str = r"ThabangVision\Collaboration\kopanod.exe";
/// The shared `kopanod` and its start-at-sign-in unit from the Linux packages.
const LINUX_SHARED: &str = "/usr/bin/kopanod";
const LINUX_UNIT_NAME: &str = "thabangvision-collab.service";
const LINUX_UNIT: &str = "/usr/lib/systemd/user/thabangvision-collab.service";
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

/// The `kopanod` to use: `KOPANO_BIN`, else the computer's shared one, else next to this program.
pub fn find_kopanod() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KOPANO_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = shared_kopanod() {
        return Some(p);
    }
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(EXE);
    p.is_file().then_some(p)
}

/// The one `kopanod` an installer set up for every app on this computer, with its own start at
/// sign-in (Windows MSI, Linux package). `None` on macOS, for a portable copy or an AppImage alone.
fn shared_kopanod() -> Option<PathBuf> {
    if std::env::var_os("KOPANO_BIN").is_some_and(|p| !p.is_empty()) {
        return None;
    }
    let p = if cfg!(windows) {
        let pf = ["ProgramW6432", "ProgramFiles"].iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))?;
        PathBuf::from(pf).join(WINDOWS_SHARED)
    } else if cfg!(target_os = "linux") {
        if !Path::new(LINUX_UNIT).exists() {
            return None;
        }
        PathBuf::from(LINUX_SHARED)
    } else {
        return None;
    };
    p.is_file().then_some(p)
}

/// Is something answering on the service's port?
pub fn answers(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Where a per-user start-at-sign-in can point: a path that stays put. An AppImage runs from a
/// temporary mount, so its `kopanod` is copied into the service folder first; a Mac app still on
/// its disk image or not yet moved out of quarantine (App Translocation) gets no autostart until
/// it is. Windows: none (only the MSI's shared copy starts at sign-in; a portable copy can be
/// moved or deleted, which would leave a broken startup entry behind).
pub fn lasting_copy(exe: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    let s = exe.to_string_lossy();
    if cfg!(target_os = "macos") && (s.contains("/AppTranslocation/") || s.starts_with("/Volumes/")) {
        return Ok(None);
    }
    if cfg!(windows) {
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

/// The per-user start-at-sign-in entry kopanod writes, as text (`None`: there is none).
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

/// The program a LaunchAgent starts: the first string of its ProgramArguments.
pub fn plist_program(plist: &str) -> Option<PathBuf> {
    let rest = &plist[plist.find("<key>ProgramArguments</key>")?..];
    let start = rest.find("<string>")? + "<string>".len();
    let len = rest[start..].find("</string>")?;
    let s = rest[start..start + len].replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&apos;", "'").replace("&quot;", "\"");
    Some(PathBuf::from(s))
}

/// The version of a `kopanod` inside a Mac app (`Contents/Resources/kopanod.version`, written by
/// the packaging), as numbers. Unknown is empty: older than any known version.
pub fn bundled_version(exe: &Path) -> Vec<u64> {
    exe.parent()
        .and_then(Path::parent)
        .and_then(|c| std::fs::read_to_string(c.join("Resources/kopanod.version")).ok())
        .map(|v| parse_version(&v))
        .unwrap_or_default()
}

/// "0.0.1.65" as [0, 0, 1, 65]; stops at the first part that is not a number.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim().split('.').map_while(|p| p.parse().ok()).collect()
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

/// Run a command and wait (at most `limit`).
fn wait_for(mut c: Command, what: &str, limit: Duration) -> Result<String, String> {
    let mut child = hidden(&mut c).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not start {what}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("{what} took too long"));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Run `kopanod <args>` and wait (at most `limit`), for init and autostart.
fn run_once(exe: &Path, args: &[&str], cfg: &(PathBuf, u16), limit: Duration) -> Result<String, String> {
    let mut c = Command::new(exe);
    c.args(args).arg("--home").arg(&cfg.0).args(["--api-port", &cfg.1.to_string()]);
    wait_for(c, &format!("kopanod {}", args.join(" ")), limit)
}

/// How `kopanod` gets started when the person signs in.
#[derive(Debug, PartialEq, Eq)]
enum AtSignIn {
    /// The installer's entry (Windows MSI, Linux package unit).
    Installer,
    /// A per-user entry `kopanod autostart` made (macOS, AppImage).
    PerUser,
    /// None (a portable copy, a Mac app still on its disk image, or turned off).
    No,
}

/// The whole check (see the module docs).
pub fn ensure_running(config: &Config) -> Result<(), String> {
    let Some(mine) = find_kopanod() else {
        log::info!("collaboration service: no kopanod next to this app (a developer build); not starting it");
        return Ok(());
    };
    let home = config.home.clone().ok_or("the collaboration folder is unknown here (set KOPANO_HOME)")?;
    let port = config.port;
    std::fs::create_dir_all(&home).map_err(|e| format!("could not make {}: {e}", home.display()))?;
    let cfg = (home.clone(), port);
    let (exe, at_sign_in) = if shared_kopanod().as_deref() == Some(mine.as_path()) {
        // the installer starts it at sign-in; a per-user entry from an older build would start a
        // second one (or a removed file), so it goes
        if autostart_entry().is_some() {
            match run_once(&mine, &["autostart", "off"], &cfg, Duration::from_secs(30)) {
                Ok(_) => log::info!("collaboration service: removed the old per-user start at sign-in"),
                Err(e) => log::warn!("collaboration service: {e}"),
            }
        }
        (mine, AtSignIn::Installer)
    } else {
        per_user(mine, &home, port, &cfg)
    };
    if !answers(port) {
        let id = run_once(&exe, &["init"], &cfg, Duration::from_secs(30))?;
        log::info!("collaboration service: this computer is {}", id.get(..8).unwrap_or(&id));
    }
    // Linux: the packaged unit is started through systemd, so it is the one that runs
    if at_sign_in == AtSignIn::Installer && cfg!(target_os = "linux") && !answers(port) {
        let mut c = Command::new("systemctl");
        c.args(["--user", "start", LINUX_UNIT_NAME]);
        if let Err(e) = wait_for(c, "systemctl --user start", Duration::from_secs(10)) {
            log::warn!("collaboration service: {e}");
        }
    }
    // macOS and Linux start it with the autostart entry; give that a moment
    let wait = if at_sign_in != AtSignIn::No && !cfg!(windows) { 3 } else { 0 };
    let start = Instant::now();
    while !answers(port) {
        if start.elapsed() > Duration::from_secs(wait) {
            return spawn_run(&exe, &home, port);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Start at sign-in for this person (no installer did it): returns the `kopanod` to run.
fn per_user(mine: PathBuf, home: &Path, port: u16, cfg: &(PathBuf, u16)) -> (PathBuf, AtSignIn) {
    let mine = match lasting_copy(&mine, home) {
        Ok(Some(p)) => p,
        Ok(None) => return (mine, AtSignIn::No),
        Err(e) => {
            log::warn!("collaboration service: {e}");
            return (mine, AtSignIn::No);
        }
    };
    if off("UBUNYE_COLLAB_AUTOSTART") {
        return (mine, AtSignIn::No);
    }
    let entry = autostart_entry();
    if let Some(entry) = &entry {
        if cfg!(target_os = "macos") {
            // another Ubunye app's kopanod is as good when it is there and at least as new
            if let Some(theirs) = plist_program(entry).filter(|p| p.is_file()) {
                if entry_points_at(entry, &theirs, home, port) && bundled_version(&theirs) >= bundled_version(&mine) {
                    return (theirs, AtSignIn::PerUser);
                }
            }
        } else if entry_points_at(entry, &mine, home, port) {
            return (mine, AtSignIn::PerUser);
        }
    }
    match run_once(&mine, &["autostart", "on"], cfg, Duration::from_secs(30)) {
        Ok(_) => {
            log::info!("collaboration service: starts when you sign in to this computer");
            (mine, AtSignIn::PerUser)
        }
        Err(e) => {
            log::warn!("collaboration service: start at sign-in not set: {e}");
            (mine, AtSignIn::No)
        }
    }
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
    fn the_launch_agent_names_its_program() {
        let plist = "<plist version=\"1.0\"><dict>\n<key>Label</key><string>org.ubunye.kopano</string>\n<key>ProgramArguments</key><array><string>/Applications/Ubunye Photo.app/Contents/MacOS/kopanod</string><string>run</string></array>\n</dict></plist>";
        assert_eq!(plist_program(plist), Some(PathBuf::from("/Applications/Ubunye Photo.app/Contents/MacOS/kopanod")));
        assert_eq!(plist_program("<plist/>"), None);
        assert_eq!(plist_program("<key>ProgramArguments</key><array><string>/A &amp; B/kopanod</string>"), Some(PathBuf::from("/A & B/kopanod")));
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(parse_version("0.0.1.65\n"), vec![0, 0, 1, 65]);
        assert!(parse_version("0.0.1.100") > parse_version("0.0.1.65"));
        assert!(parse_version("0.1.0.1") > parse_version("0.0.9.900"));
        // unknown (an older app without the file) is older than anything known
        assert!(Vec::<u64>::new() < parse_version("0.0.1.1"));
        assert!(bundled_version(Path::new("/no/such/app/Contents/MacOS/kopanod")).is_empty());
    }

    #[test]
    fn switches_read_plain_words() {
        assert!(!off("UBUNYE_COLLAB_TEST_UNSET_VARIABLE"));
    }

    #[test]
    fn a_missing_kopanod_is_not_an_error() {
        let cfg = Config { port: 1, home: Some(std::env::temp_dir().join("ubunye-collab-test-none")) };
        // the test binary has no kopanod next to it (and this computer may have a shared one)
        if find_kopanod().is_none() {
            assert_eq!(ensure_running(&cfg), Ok(()));
        }
    }
}
