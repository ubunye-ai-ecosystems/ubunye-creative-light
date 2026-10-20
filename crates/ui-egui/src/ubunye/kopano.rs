//! Client for `kopanod`, the Kopano service on this computer (Ubunye, Kopano slice S3b).
//!
//! One service per computer holds the device identity and does all the syncing; Light never syncs
//! itself. Light only talks to the service's local API: TCP on 127.0.0.1, one JSON request per
//! line, each carrying the token from `<home>/api.token`. The service drives Light back through
//! Light's own control port (`docs/control-protocol.md`).
//!
//! Everything here runs off the UI thread: one thread polls `hello` / `status` every few seconds,
//! one holds a `subscribe` stream of events, and Share / Join each run on a thread of their own.
//! The UI reads a [`State`] snapshot behind a mutex and never waits for the service. When the
//! service stops, both threads keep retrying, so a restarted service is found again by itself.
//!
//! Defaults shared with `kopanod`: API port `KOPANO_API_PORT`, else 47100; home `KOPANO_HOME`,
//! else `%LOCALAPPDATA%\Kopano` (Windows), `~/Library/Application Support/Kopano` (macOS),
//! `~/.local/share/kopano` (Linux, or `$XDG_DATA_HOME/kopano`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::{Value, json};

/// The service's API port when `KOPANO_API_PORT` doesn't say otherwise.
pub const DEFAULT_API_PORT: u16 = 47100;
/// How often the poll thread asks the service for `hello` and `status`.
pub const POLL_MS: u64 = 3000;
/// Recent activity lines kept (newest first).
pub const ACTIVITY_KEEP: usize = 40;
/// After an `attach`, the service's errors about the old port are stale for this long (ms).
pub const RELINK_QUIET_MS: f64 = 15000.0;
/// What the panel says when the service doesn't answer.
pub const NOT_RUNNING: &str = "Collaboration is not running on this computer";
/// Longest line read from the service (its replies and events are small JSON objects).
const MAX_LINE: usize = 1 << 20;

/// Where the service lives: its API port and its home folder (which holds `api.token`).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub port: u16,
    pub home: Option<PathBuf>,
}

impl Config {
    /// From this process's environment (`KOPANO_API_PORT`, `KOPANO_HOME`, the OS folders).
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok();
        Config { port: api_port_with(get), home: home_with(get, std::env::consts::OS) }
    }
}

/// The API port: `KOPANO_API_PORT` when it is a port number, else [`DEFAULT_API_PORT`].
pub fn api_port_with(get: impl Fn(&str) -> Option<String>) -> u16 {
    match get("KOPANO_API_PORT").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(v) => match v.parse::<u16>() {
            Ok(p) if p != 0 => p,
            _ => {
                log::warn!("KOPANO_API_PORT {v:?} is not a port number; using {DEFAULT_API_PORT}");
                DEFAULT_API_PORT
            }
        },
        None => DEFAULT_API_PORT,
    }
}

/// The service's home folder for `os` (`std::env::consts::OS`), with `get` reading the
/// environment. `None` when the variables it needs are not set.
pub fn home_with(get: impl Fn(&str) -> Option<String>, os: &str) -> Option<PathBuf> {
    let var = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(h) = var("KOPANO_HOME") {
        return Some(h);
    }
    match os {
        "windows" => var("LOCALAPPDATA").map(|d| d.join("Kopano")),
        "macos" => var("HOME").map(|h| h.join("Library").join("Application Support").join("Kopano")),
        _ => var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share"))).map(|d| d.join("kopano")),
    }
}

/// The API token the service wrote into its home folder.
pub fn read_token(home: &Path) -> Result<String, String> {
    let f = home.join("api.token");
    match std::fs::read_to_string(&f) {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        Ok(_) => Err(format!("the collaboration key file is empty ({})", f.display())),
        Err(_) => Err(format!("the collaboration key file is missing ({})", f.display())),
    }
}

/// The first 8 characters of a device or project id (ids are hex, so this is a char boundary).
pub fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// A 64-character hex id (devices; projects).
pub fn is_hex_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Who this computer is signed in as.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Me {
    pub device: String,
    pub name: String,
    pub account: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Teammate {
    pub device: String,
    pub account: Option<String>,
    /// Known once they were online (the service names offline teammates by device id only).
    pub name: Option<String>,
    pub online: bool,
}

impl Teammate {
    /// The name, else the short device id.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| short(&self.device).to_string())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub head: Option<String>,
    pub teammates: Vec<Teammate>,
    /// The Light control port the service drives this project through (`light-control` only).
    pub app_port: Option<u16>,
    /// Something answers on `app_port` (checked by the poll thread; `None` = not checked).
    pub app_alive: Option<bool>,
    pub members: Vec<Member>,
    /// "live" (every edit goes to everyone) or "approve" (the owner approves changes); `None`
    /// from an older service, which only knows live.
    pub mode: Option<String>,
    /// What waits for approval, and where this person's own work is (approve mode).
    pub review: Option<Review>,
    /// Settings both sides changed at once in a live library, waiting for a pick.
    pub clashes: Vec<Clash>,
}

/// An "I approve changes" library as the service sees it (`status.review`, `review` events).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub is_owner: bool,
    /// This person may approve (the owner, or someone the owner chose).
    pub approver: bool,
    pub owner_name: String,
    pub pending: Vec<Pending>,
    /// Members whose latest work was declined (their branch keeps it).
    pub declined: Vec<String>,
    /// This person's own work (a member): none, waiting, approved or declined.
    pub mine: Option<Mine>,
    /// The owner's window shows a member's version now.
    pub viewing: Option<Viewing>,
}

/// One teammate's changes waiting for review.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Pending {
    pub account: String,
    pub name: String,
    pub version: String,
    pub photos: u64,
    /// Photo keys (content hashes) they changed.
    pub changed: Vec<String>,
    pub clashes: Vec<PendingClash>,
}

/// A setting the member and main both changed since the member last caught up.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PendingClash {
    pub key: String,
    pub photo: String,
    pub setting: String,
    pub main: String,
    pub member: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Mine {
    pub state: String,
    pub photos: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Viewing {
    pub account: String,
    pub name: String,
    /// "member" (their version shows) or "main" (before).
    pub side: String,
}

/// A live library's clash: one setting of one photo, two values; a person picks one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Clash {
    /// The service's id for it (what `resolve` takes).
    pub id: String,
    pub photo: String,
    pub setting: String,
    pub a: String,
    pub b: String,
}

/// A setting value in plain words: numbers with two decimals, text as it is.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map(|f| if f.fract() == 0.0 { format!("{f:.0}") } else { format!("{f:.2}") }).unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        Value::Null => "none".to_string(),
        Value::Bool(b) => if *b { "on" } else { "off" }.to_string(),
        other => other.to_string(),
    }
}

/// "light.exposure" -> "Exposure"; "rating" -> "Rating" (the last part, in words).
pub fn setting_text(path: &str) -> String {
    let last = path.rsplit('.').next().unwrap_or(path).replace('_', " ");
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => last,
    }
}

impl Review {
    /// From `status.review` or a `review` event.
    pub fn from_value(v: &Value) -> Review {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let pending = v
            .get("pending")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|e| Pending {
                account: s(e, "account"),
                name: s(e, "name"),
                version: s(e, "version"),
                photos: e.get("photos").and_then(Value::as_u64).unwrap_or(0),
                changed: e.get("changes").and_then(Value::as_array).into_iter().flatten().map(|c| s(c, "photo")).collect(),
                clashes: e
                    .get("clashes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|c| {
                        let (key, photo) = (s(c, "key"), s(c, "photo"));
                        c.get("settings").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(move |x| PendingClash {
                            key: key.clone(),
                            photo: photo.clone(),
                            setting: s(&x, "setting"),
                            main: value_text(x.get("main").unwrap_or(&Value::Null)),
                            member: value_text(x.get("member").unwrap_or(&Value::Null)),
                        })
                    })
                    .collect(),
            })
            .collect();
        Review {
            is_owner: v.get("is_owner").and_then(Value::as_bool).unwrap_or(false),
            approver: v.get("approver").and_then(Value::as_bool).unwrap_or(false),
            owner_name: s(v, "owner_name"),
            pending,
            declined: v.get("declined").and_then(Value::as_array).into_iter().flatten().map(|d| s(d, "name")).collect(),
            mine: v
                .get("mine")
                .filter(|m| m.is_object())
                .map(|m| Mine { state: s(m, "state"), photos: m.get("photos").and_then(Value::as_u64).unwrap_or(0) }),
            viewing: v.get("viewing").filter(|m| m.is_object()).map(|m| Viewing { account: s(m, "account"), name: s(m, "name"), side: s(m, "side") }),
        }
    }

    /// The member's line: "Waiting for Alice's approval", "Approved", "Declined".
    pub fn mine_text(&self) -> Option<String> {
        let m = self.mine.as_ref()?;
        let owner = if self.owner_name.is_empty() { "the owner".to_string() } else { self.owner_name.clone() };
        Some(match m.state.as_str() {
            "waiting" => format!("You changed {} · Waiting for {owner}'s approval", photos_text(Some(m.photos))),
            "approved" => format!("Approved: {owner} took your changes"),
            "declined" => format!("Declined: {owner} kept the library as it was. Your changes stay in your window."),
            _ => format!("Your changes go to {owner} for approval"),
        })
    }
}

/// From `status.clashes` (live libraries).
pub fn clashes_from(v: &Value) -> Vec<Clash> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    v.as_array()
        .into_iter()
        .flatten()
        .flat_map(|c| {
            let (id, photo) = (s(c, "clash"), s(c, "photo"));
            c.get("settings").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().take(1).map(move |x| Clash {
                id: id.clone(),
                photo: photo.clone(),
                setting: s(&x, "setting"),
                a: value_text(x.get("a").unwrap_or(&Value::Null)),
                b: value_text(x.get("b").unwrap_or(&Value::Null)),
            })
        })
        .collect()
}

/// Who may join a project (from `status`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Member {
    pub account: String,
    pub role: String,
    pub owner: bool,
}

/// What happened, kept as data so the UI words it at display time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Said {
    Online {
        who: String,
    },
    Offline {
        who: String,
    },
    /// An edit made in this window was saved and sent.
    YouChanged {
        photos: Option<u64>,
    },
    /// A teammate's edit arrived and was applied here (`photos`: how many, when the service says).
    Changed {
        who: String,
        photos: Option<u64>,
    },
    /// Both sides edited at once; their edits were merged (clashes: same setting on both sides).
    Merged {
        who: String,
        clashes: usize,
    },
    /// This window was linked to a shared library again (Light had restarted on a new port).
    Reattached {
        name: String,
    },
    /// Both sides ended up with the same photos.
    UpToDate {
        who: String,
    },
    /// The service could not read or write this window.
    AppProblem {
        error: String,
    },
    /// A teammate's changes wait for this person's approval.
    ToReview {
        who: String,
        photos: u64,
    },
    /// The owner (or an approver) approved this person's changes.
    Approved {
        who: String,
    },
    /// ... or declined them.
    Declined {
        who: String,
    },
}

impl Said {
    /// In plain words (English; the activity sentences are not in the catalogs yet).
    pub fn text(&self) -> String {
        match self {
            Said::Online { who } => format!("{who} is online"),
            Said::Offline { who } => format!("{who} went offline"),
            Said::YouChanged { photos } => format!("You changed {}", photos_text(*photos)),
            Said::Changed { who, photos } => format!("{who} changed {}", photos_text(*photos)),
            Said::Merged { who, clashes: 0 } => format!("Your changes and {who}'s were put together"),
            Said::Merged { who, clashes: 1 } => format!("Your changes and {who}'s were put together · 1 clash"),
            Said::Merged { who, clashes } => format!("Your changes and {who}'s were put together · {clashes} clashes"),
            Said::UpToDate { who } => format!("Up to date with {who}"),
            Said::Reattached { name } => format!("This window is linked to {name} again"),
            // the service's own words stay in `kopano.status` and the log, not on screen
            Said::AppProblem { .. } => "Lost touch with this window for a moment; it reconnects by itself".to_string(),
            Said::ToReview { who, photos } => format!("{who} changed {} · waiting for your review", photos_text(Some(*photos))),
            Said::Approved { who } => format!("{who} approved your changes"),
            Said::Declined { who } => format!("{who} declined your changes"),
        }
    }
}

/// "2 photos", "1 photo", or "photos" when the count is unknown.
fn photos_text(n: Option<u64>) -> String {
    match n {
        Some(1) => "1 photo".to_string(),
        Some(n) => format!("{n} photos"),
        None => "photos".to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Activity {
    /// Wall-clock ms of the latest time it happened.
    pub at_ms: f64,
    pub project: Option<String>,
    pub said: Said,
    /// The same thing several times in a row (a slider drag is many edits).
    pub times: u32,
}

/// The last Share or Join.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Job {
    pub busy: bool,
    pub ok: bool,
    pub text: String,
    /// The project a Share made (to copy and send to the teammate).
    pub project: Option<String>,
}

/// Everything the panel shows. The client threads write it; the UI reads a copy each frame.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The client threads were started.
    pub started: bool,
    /// The service answered the last poll.
    pub running: bool,
    /// This computer is signed in to ThabangVision (the service's `hello`).
    pub signed_in: bool,
    /// Until when (wall-clock ms), when the service says.
    pub signed_in_until_ms: Option<f64>,
    /// Why it isn't running, or what went wrong talking to it (plain words).
    pub problem: Option<String>,
    pub me: Option<Me>,
    pub projects: Vec<Project>,
    /// Newest first.
    pub activity: Vec<Activity>,
    pub job: Option<Job>,
    /// Device id → name, learned from `status` and events (offline teammates keep their names).
    #[serde(skip)]
    pub names: BTreeMap<String, String>,
}

impl State {
    fn name_of(&self, device: &str) -> String {
        self.names.get(device).cloned().unwrap_or_else(|| short(device).to_string())
    }

    /// Add a line; the same thing again right after the last one only bumps its time and count.
    pub fn note(&mut self, project: Option<String>, said: Said, now_ms: f64) {
        if let Some(last) = self.activity.first_mut()
            && last.said == said
            && last.project == project
        {
            last.at_ms = now_ms;
            last.times = last.times.saturating_add(1);
            return;
        }
        self.activity.insert(0, Activity { at_ms: now_ms, project, said, times: 1 });
        self.activity.truncate(ACTIVITY_KEEP);
    }

    /// Take the reply to `hello` (who we are, which projects).
    pub fn apply_hello(&mut self, v: &Value) {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        self.me = Some(Me { device: s("device"), name: s("name"), account: s("account") });
        self.signed_in = v.get("signed_in").and_then(Value::as_bool).unwrap_or(false);
        self.signed_in_until_ms = v.get("signed_in_until_ms").and_then(Value::as_f64);
        let mut projects = Vec::new();
        for p in v.get("projects").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            // keep what the last status said until the next one
            let old = self.projects.iter().find(|o| o.id == id).cloned().unwrap_or_default();
            let app_port = app_port(p.get("app"));
            projects.push(Project {
                id: id.to_string(),
                name,
                app_alive: if app_port == old.app_port { old.app_alive } else { None },
                app_port,
                ..old
            });
        }
        projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
        self.projects = projects;
    }

    /// Take the reply to `status {project}`.
    pub fn apply_status(&mut self, project: &str, v: &Value) {
        let mut online: BTreeMap<String, String> = v
            .get("online")
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(d, n)| (d.clone(), n.as_str().unwrap_or_default().to_string())).collect())
            .unwrap_or_default();
        for (d, n) in &online {
            if !n.is_empty() {
                self.names.insert(d.clone(), n.clone());
            }
        }
        let mine = self.me.as_ref().map(|m| m.device.clone()).unwrap_or_default();
        // teammates: device ids (older services) or {device, name, account, online}
        let mut accounts: BTreeMap<String, String> = BTreeMap::new();
        let mut devices: Vec<String> = Vec::new();
        for t in v.get("teammates").and_then(Value::as_array).into_iter().flatten() {
            let Some(d) = t.as_str().or_else(|| t.get("device").and_then(Value::as_str)) else { continue };
            if let Some(n) = t.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                self.names.insert(d.to_string(), n.to_string());
            }
            if let Some(a) = t.get("account").and_then(Value::as_str) {
                accounts.insert(d.to_string(), a.to_string());
            }
            if t.get("online").and_then(Value::as_bool) == Some(true) && !online.contains_key(d) {
                online.insert(d.to_string(), self.names.get(d).cloned().unwrap_or_default());
            }
            devices.push(d.to_string());
        }
        for d in online.keys() {
            if !devices.contains(d) {
                devices.push(d.clone());
            }
        }
        devices.retain(|d| *d != mine);
        let names = &self.names;
        let teammates = devices
            .iter()
            .map(|d| Teammate { device: d.clone(), account: accounts.get(d).cloned(), name: names.get(d).cloned(), online: online.contains_key(d) })
            .collect::<Vec<_>>();
        let head = v.get("head").and_then(Value::as_str).map(str::to_string);
        let members: Vec<Member> = v
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(Member {
                    account: m.get("account")?.as_str()?.to_string(),
                    role: m.get("role").and_then(Value::as_str).unwrap_or("editor").to_string(),
                    owner: m.get("owner").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        let port = v.get("app").map(|a| app_port(Some(a)));
        let mode = v.get("mode").and_then(Value::as_str).map(str::to_string);
        let review = v.get("review").map(Review::from_value);
        let clashes = v.get("clashes").map(clashes_from).unwrap_or_default();
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == project) {
            p.teammates = teammates;
            p.head = head;
            p.members = members;
            p.mode = mode;
            p.review = review;
            p.clashes = clashes;
            if let Some(port) = port
                && port != p.app_port
            {
                p.app_port = port;
                p.app_alive = None;
            }
        }
    }

    /// Take one event line from `subscribe`.
    pub fn apply_event(&mut self, ev: &Value, now_ms: f64) {
        let s = |k: &str| ev.get(k).and_then(Value::as_str);
        let project = s("project").map(str::to_string);
        match s("event") {
            Some("teammate") => {
                let Some(device) = s("device") else { return };
                if let Some(n) = s("name").filter(|n| !n.is_empty()) {
                    self.names.insert(device.to_string(), n.to_string());
                }
                let online = ev.get("online").and_then(Value::as_bool).unwrap_or(false);
                let name = self.names.get(device).cloned();
                if let Some(p) = project.as_ref().and_then(|id| self.projects.iter_mut().find(|p| p.id == *id)) {
                    match p.teammates.iter_mut().find(|t| t.device == device) {
                        Some(t) => {
                            t.online = online;
                            t.name = name.or(t.name.take());
                        }
                        None => p.teammates.push(Teammate { device: device.to_string(), account: None, name, online }),
                    }
                }
                let who = self.name_of(device);
                self.note(project, if online { Said::Online { who } } else { Said::Offline { who } }, now_ms);
            }
            Some("saved") => {
                self.set_head(project.as_deref(), s("head"));
                self.note(project, Said::YouChanged { photos: photo_count(ev) }, now_ms);
            }
            Some("synced") => {
                self.set_head(project.as_deref(), s("head"));
                let who = s("from").unwrap_or("A teammate").to_string();
                let clashes = ev.get("clashes").and_then(Value::as_u64).map(|c| usize::try_from(c).unwrap_or(usize::MAX)).unwrap_or(0);
                let said = match s("how") {
                    Some("merged") => Said::Merged { who, clashes },
                    Some(h) if h.starts_with("same content") => Said::UpToDate { who },
                    _ => Said::Changed { who, photos: photo_count(ev) },
                };
                self.note(project, said, now_ms);
            }
            Some("review") => {
                let new = Review::from_value(ev);
                let Some(p) = project.as_ref().and_then(|id| self.projects.iter_mut().find(|p| p.id == *id)) else { return };
                let old = p.review.replace(new.clone()).unwrap_or_default();
                p.mode = Some("approve".to_string());
                // say what changed: new work to review, or what happened to mine
                let mut said = vec![];
                for e in &new.pending {
                    if !old.pending.iter().any(|o| o.account == e.account && o.version == e.version) {
                        said.push(Said::ToReview { who: e.name.clone(), photos: e.photos });
                    }
                }
                let (was, now) = (old.mine.map(|m| m.state), new.mine.as_ref().map(|m| m.state.clone()));
                if was.is_some() && was != now {
                    match now.as_deref() {
                        Some("approved") => said.push(Said::Approved { who: new.owner_name.clone() }),
                        Some("declined") => said.push(Said::Declined { who: new.owner_name.clone() }),
                        _ => {}
                    }
                }
                for x in said {
                    self.note(project.clone(), x, now_ms);
                }
            }
            Some("app") => {
                let error = s("error").unwrap_or("unknown problem").to_string();
                log::info!("collaboration: the service could not reach this window: {error}");
                // the service's last try at the old port, just after this window linked it again
                let just_linked = self
                    .activity
                    .iter()
                    .any(|a| a.project == project && matches!(a.said, Said::Reattached { .. }) && now_ms - a.at_ms < RELINK_QUIET_MS);
                if just_linked {
                    return;
                }
                self.note(project, Said::AppProblem { error }, now_ms);
            }
            _ => {}
        }
    }

    fn set_head(&mut self, project: Option<&str>, head: Option<&str>) {
        // events carry the short head; status the full one: both only say "it moved"
        if let (Some(id), Some(h)) = (project, head)
            && let Some(p) = self.projects.iter_mut().find(|p| p.id == id)
        {
            p.head = Some(h.to_string());
        }
    }
}

/// The control port of a `light-control` app binding (`{"kind": "light-control", "port": N}`).
pub fn app_port(app: Option<&Value>) -> Option<u16> {
    let a = app?;
    if a.get("kind").and_then(Value::as_str) != Some("light-control") {
        return None;
    }
    a.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0)
}

/// Where this app remembers which photos a shared library holds, so that after a restart on a
/// new control port it knows which libraries are its own (`<home>/light/<project>.json`).
pub fn record_path(home: &Path, project: &str) -> Option<PathBuf> {
    is_hex_id(project).then(|| home.join("light").join(format!("{project}.json")))
}

pub fn write_record(home: &Path, project: &str, photos: &std::collections::BTreeSet<String>) -> Result<(), String> {
    let f = record_path(home, project).ok_or("not a project id")?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, json!({"photos": photos}).to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

/// The photos recorded for `project` (content hashes); empty when there is no readable record.
pub fn read_record(home: &Path, project: &str) -> std::collections::BTreeSet<String> {
    let Some(f) = record_path(home, project) else { return Default::default() };
    let Ok(text) = std::fs::read_to_string(f) else { return Default::default() };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v.get("photos").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// This library holds a shared library when at least half of the photos recorded for it are here.
pub fn holds(recorded: &std::collections::BTreeSet<String>, ours: &std::collections::BTreeSet<String>) -> bool {
    !recorded.is_empty() && recorded.intersection(ours).count().saturating_mul(2) >= recorded.len()
}

/// How many photos an event says changed (`photos`), if it says.
fn photo_count(ev: &Value) -> Option<u64> {
    ev.get("photos").and_then(Value::as_u64).filter(|n| *n > 0)
}

/// "3 s ago", "2 min ago" (`secs` since it happened).
pub fn ago(secs: f64) -> String {
    let s = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    if s < 2.0 {
        "just now".to_string()
    } else if s < 60.0 {
        format!("{} s ago", s.floor())
    } else if s < 3600.0 {
        format!("{} min ago", (s / 60.0).floor())
    } else if s < 86400.0 {
        format!("{} h ago", (s / 3600.0).floor())
    } else {
        format!("{} d ago", (s / 86400.0).floor())
    }
}

/// Shared between the client threads and the UI.
pub type Shared = Arc<Mutex<State>>;

pub fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The client: its state and how to wake the poll thread.
pub struct Client {
    pub config: Config,
    pub shared: Shared,
    #[cfg(not(target_arch = "wasm32"))]
    wake: Option<std::sync::mpsc::Sender<()>>,
}

impl Client {
    pub fn new(config: Config) -> Self {
        Client {
            config,
            shared: Arc::default(),
            #[cfg(not(target_arch = "wasm32"))]
            wake: None,
        }
    }

    /// A copy of the state for this frame.
    pub fn state(&self) -> State {
        lock(&self.shared).clone()
    }

    pub fn started(&self) -> bool {
        lock(&self.shared).started
    }
}

#[cfg(target_arch = "wasm32")]
impl Client {
    pub fn start(&mut self, _ctx: &egui::Context) {
        let mut s = lock(&self.shared);
        s.started = true;
        s.problem = Some("Collaboration works in the desktop app only.".to_string());
    }

    pub fn request(&mut self, _ctx: &egui::Context, _busy: &str, _req: Value) -> Result<(), String> {
        Err("Collaboration works in the desktop app only.".to_string())
    }

    pub fn attach(&mut self, _ctx: &egui::Context, _project: &str, _name: &str, _port: u16) {}
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{call, connect};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    use super::*;

    /// One connection to the service's API.
    pub struct Conn {
        read: BufReader<TcpStream>,
        write: TcpStream,
        token: String,
    }

    /// Connect and read the token. The error is in plain words for the panel.
    pub fn connect(config: &Config, timeout: Duration) -> Result<Conn, String> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).map_err(|_| NOT_RUNNING.to_string())?;
        let Some(home) = &config.home else { return Err("the collaboration folder is unknown here (set KOPANO_HOME)".to_string()) };
        let token = read_token(home)?;
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_nodelay(true);
        let write = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Conn { read: BufReader::new(stream), write, token })
    }

    impl Conn {
        /// Send one request (the token is added) without reading the reply.
        fn send(&mut self, mut req: Value) -> Result<(), String> {
            if let Some(o) = req.as_object_mut() {
                o.insert("token".into(), json!(self.token));
            }
            writeln!(self.write, "{req}").and_then(|()| self.write.flush()).map_err(|_| "the collaboration service stopped answering".to_string())
        }

        /// One line from the service; `Ok(None)` when it hung up.
        pub fn line(&mut self) -> Result<Option<Value>, String> {
            let mut buf = Vec::new();
            let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX);
            let n =
                (&mut self.read).take(limit).read_until(b'\n', &mut buf).map_err(|_| "the collaboration service stopped answering".to_string())?;
            if n == 0 {
                return Ok(None);
            }
            serde_json::from_slice(&buf).map(Some).map_err(|_| "the collaboration service sent something unreadable".to_string())
        }
    }

    /// One request, one reply: the `result`, or the service's error.
    pub fn call(conn: &mut Conn, req: Value) -> Result<Value, String> {
        conn.send(req)?;
        let reply = conn.line()?.ok_or_else(|| "the collaboration service hung up".to_string())?;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
        let e = reply.get("error").and_then(Value::as_str).unwrap_or("the collaboration service refused").to_string();
        Err(if e == "bad token" { "the collaboration service refused this window (its key file api.token does not match)".to_string() } else { e })
    }

    /// Ask `hello` and every project's `status` once.
    pub fn poll_once(config: &Config, shared: &Shared) {
        let r = (|| -> Result<(), String> {
            let mut c = connect(config, Duration::from_secs(5))?;
            let hello = call(&mut c, json!({"op": "hello"}))?;
            let ids: Vec<String> = {
                let mut s = lock(shared);
                s.apply_hello(&hello);
                s.projects.iter().map(|p| p.id.clone()).collect()
            };
            for id in ids {
                // one project's trouble doesn't hide the others
                if let Ok(st) = call(&mut c, json!({"op": "status", "project": id})) {
                    lock(shared).apply_status(&id, &st);
                }
            }
            // is a Light still answering where each project points? (a dead port after a restart)
            let ports: Vec<(String, u16)> = lock(shared).projects.iter().filter_map(|p| Some((p.id.clone(), p.app_port?))).collect();
            for (id, port) in ports {
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                let alive = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok();
                if let Some(p) = lock(shared).projects.iter_mut().find(|p| p.id == id && p.app_port == Some(port)) {
                    p.app_alive = Some(alive);
                }
            }
            Ok(())
        })();
        let mut s = lock(shared);
        match r {
            Ok(()) => {
                s.running = true;
                s.problem = None;
            }
            Err(e) => {
                s.running = false;
                for p in &mut s.projects {
                    for t in &mut p.teammates {
                        t.online = false;
                    }
                }
                s.problem = Some(e);
            }
        }
    }

    impl Client {
        /// Start the poll and event threads (once). `ctx` is repainted when something changes.
        pub fn start(&mut self, ctx: &egui::Context) {
            if self.started() {
                return;
            }
            lock(&self.shared).started = true;
            let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
            self.wake = Some(wake_tx);
            let (config, shared, repaint) = (self.config.clone(), self.shared.clone(), ctx.clone());
            let poll = std::thread::Builder::new().name("kopano-poll".into()).spawn(move || {
                loop {
                    let before = format!("{:?}", lock(&shared).projects);
                    let was = lock(&shared).running;
                    poll_once(&config, &shared);
                    let s = lock(&shared);
                    if s.running != was || format!("{:?}", s.projects) != before {
                        repaint.request_repaint();
                    }
                    drop(s);
                    match wake_rx.recv_timeout(Duration::from_millis(POLL_MS)) {
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        // the app is gone
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
            let (config, shared, repaint) = (self.config.clone(), self.shared.clone(), ctx.clone());
            let events = std::thread::Builder::new().name("kopano-events".into()).spawn(move || {
                loop {
                    let _ = follow_events(&config, &shared, &repaint);
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
            if let Err(e) = poll.and(events) {
                lock(&self.shared).problem = Some(format!("could not start collaboration: {e}"));
            }
        }

        /// Ask the poll thread to look now (after a Share or Join).
        pub fn wake(&self) {
            if let Some(w) = &self.wake {
                let _ = w.send(());
            }
        }

        /// Point `project` at this window's control `port` again (`attach`), quietly: the outcome
        /// is an activity line, not the Share / Join line.
        pub fn attach(&mut self, ctx: &egui::Context, project: &str, name: &str, port: u16) {
            self.start(ctx);
            let (config, shared, repaint, wake) = (self.config.clone(), self.shared.clone(), ctx.clone(), self.wake.clone());
            let req = json!({"op": "attach", "project": project, "app": {"kind": "light-control", "port": port}});
            let (project, name) = (project.to_string(), name.to_string());
            let _ = std::thread::Builder::new().name("kopano-attach".into()).spawn(move || {
                let r = connect(&config, Duration::from_secs(20)).and_then(|mut c| call(&mut c, req));
                let said = match r {
                    Ok(_) => Said::Reattached { name },
                    Err(error) => Said::AppProblem { error },
                };
                let mut s = lock(&shared);
                if matches!(said, Said::Reattached { .. }) {
                    // the trouble before (the old port) is over
                    s.activity.retain(|a| !(a.project.as_deref() == Some(project.as_str()) && matches!(a.said, Said::AppProblem { .. })));
                }
                s.note(Some(project), said, crate::now_ms());
                drop(s);
                if let Some(w) = wake {
                    let _ = w.send(());
                }
                repaint.request_repaint();
            });
        }

        /// Run one request (`create-project`, `join`…) on its own thread; the outcome lands in
        /// [`State::job`]. `busy` is what the panel says meanwhile.
        pub fn request(&mut self, ctx: &egui::Context, busy: &str, req: Value) -> Result<(), String> {
            self.start(ctx);
            {
                let mut s = lock(&self.shared);
                if s.job.as_ref().is_some_and(|j| j.busy) {
                    return Err("still busy with the last request".to_string());
                }
                s.job = Some(Job { busy: true, ok: false, text: busy.to_string(), project: None });
            }
            let (config, shared, repaint, wake) = (self.config.clone(), self.shared.clone(), ctx.clone(), self.wake.clone());
            let op = req.get("op").and_then(Value::as_str).unwrap_or_default().to_string();
            let spawned = std::thread::Builder::new().name("kopano-job".into()).spawn(move || {
                // a join fetches the project from the teammate: up to 30 s to reach them
                let r = connect(&config, Duration::from_secs(90)).and_then(|mut c| call(&mut c, req));
                let job = match r {
                    Ok(v) => job_done(&op, &v),
                    Err(e) => Job { busy: false, ok: false, text: e, project: None },
                };
                lock(&shared).job = Some(job);
                if let Some(w) = wake {
                    let _ = w.send(());
                }
                repaint.request_repaint();
            });
            if let Err(e) = spawned {
                let text = format!("could not start the request: {e}");
                lock(&self.shared).job = Some(Job { busy: false, ok: false, text: text.clone(), project: None });
                return Err(text);
            }
            Ok(())
        }
    }

    /// Hold a `subscribe` stream until the service hangs up.
    fn follow_events(config: &Config, shared: &Shared, repaint: &egui::Context) -> Result<(), String> {
        // no read timeout: events come when they come
        let mut c = connect(config, Duration::from_secs(3600 * 24 * 365))?;
        call(&mut c, json!({"op": "subscribe"}))?;
        while let Some(ev) = c.line()? {
            lock(shared).apply_event(&ev, crate::now_ms());
            repaint.request_repaint();
        }
        Ok(())
    }
}

/// What the panel says once a Share or Join is done.
pub fn job_done(op: &str, v: &Value) -> Job {
    match op {
        "create-project" => {
            let shared = match v.get("photos").and_then(Value::as_u64) {
                Some(1) => "Shared 1 photo".to_string(),
                Some(n) => format!("Shared {n} photos"),
                None => "Shared".to_string(),
            };
            Job {
                busy: false,
                ok: true,
                text: format!("{shared}. Until invites by username arrive, your teammate joins with the ids under Advanced (testing)."),
                project: v.get("project").and_then(Value::as_str).map(str::to_string),
            }
        }
        "join" => Job {
            busy: false,
            ok: true,
            text: "Joined. Your teammate's photos and edits arrive in a few seconds.".to_string(),
            project: v.get("project").and_then(Value::as_str).map(str::to_string),
        },
        "approve" => Job { busy: false, ok: true, text: "Approved. The changes are in your library.".to_string(), project: None },
        "decline" => Job { busy: false, ok: true, text: "Declined. Your library stays as it was.".to_string(), project: None },
        "view" => Job {
            busy: false,
            ok: true,
            text: match v.pointer("/viewing/side").and_then(Value::as_str) {
                Some("member") => "Showing their version of the photos they changed. Editing waits until you are done.".to_string(),
                Some("main") => "Showing your version (before).".to_string(),
                _ => "Back to your library.".to_string(),
            },
            project: None,
        },
        "resolve" => Job { busy: false, ok: true, text: "Clash settled. Everyone gets your pick.".to_string(), project: None },
        _ => Job { busy: false, ok: true, text: "Done.".to_string(), project: None },
    }
}
