//! Kopano client and panel: paths and token, events in plain words, a fake `kopanod` over real
//! loopback TCP (incl. a restart), and the panel driven through the control channel.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::kopano::*;
use super::kopano_panel::KopanoPanel;

const ALICE: &str = "c013b977c6b7b2118e21cbcb4cfd63800a5f798409997d4007fbe00b8438d344";
const M1: &str = "2635f5f796bbc17f7d7a42b10a64c795068d3cbeb4a8d6e95157b87d26300486";
const PID: &str = "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
const NEW_PID: &str = "bb11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
const TOKEN: &str = "secret-token";

fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
}

#[test]
fn home_folder_follows_kopanod() {
    let all = [("KOPANO_HOME", ""), ("LOCALAPPDATA", r"C:\Users\a\AppData\Local"), ("HOME", "/home/a"), ("XDG_DATA_HOME", "")];
    assert_eq!(home_with(env(&all), "windows"), Some(PathBuf::from(r"C:\Users\a\AppData\Local").join("Kopano")));
    assert_eq!(home_with(env(&all), "macos"), Some(PathBuf::from("/home/a/Library/Application Support/Kopano")));
    assert_eq!(home_with(env(&all), "linux"), Some(PathBuf::from("/home/a/.local/share/kopano")));
    assert_eq!(home_with(env(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")]), "linux"), Some(PathBuf::from("/data/kopano")));
    // KOPANO_HOME wins everywhere
    for os in ["windows", "macos", "linux"] {
        assert_eq!(home_with(env(&[("KOPANO_HOME", "/k"), ("HOME", "/home/a")]), os), Some(PathBuf::from("/k")));
    }
    assert_eq!(home_with(env(&[]), "windows"), None);
}

#[test]
fn api_port_follows_kopanod() {
    assert_eq!(api_port_with(env(&[])), DEFAULT_API_PORT);
    assert_eq!(DEFAULT_API_PORT, 47100);
    assert_eq!(api_port_with(env(&[("KOPANO_API_PORT", " 47121 ")])), 47121);
    for bad in ["", "nope", "0", "70000"] {
        assert_eq!(api_port_with(env(&[("KOPANO_API_PORT", bad)])), DEFAULT_API_PORT, "{bad:?}");
    }
}

static N: AtomicUsize = AtomicUsize::new(0);

fn temp_home() -> PathBuf {
    let d = std::env::temp_dir().join(format!("kopano-panel-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn token_is_read_trimmed_and_a_missing_one_is_explained() {
    let home = temp_home();
    assert!(read_token(&home).unwrap_err().contains("missing"));
    std::fs::write(home.join("api.token"), "  ").unwrap();
    assert!(read_token(&home).unwrap_err().contains("empty"));
    std::fs::write(home.join("api.token"), "abc123\n").unwrap();
    assert_eq!(read_token(&home).unwrap(), "abc123");
}

fn state_with_project() -> State {
    let mut s = State::default();
    s.apply_hello(&json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "projects": [{"id": PID, "name": "Shoot"}]}));
    s
}

#[test]
fn events_read_as_plain_words() {
    let mut s = state_with_project();
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": M1, "name": "Member1", "online": true}), 1000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward", "clashes": 0}), 2000.0);
    s.apply_event(&json!({"event": "saved", "project": PID, "head": "abcd"}), 3000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "merged", "clashes": 3}), 4000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "same content, settled", "clashes": 0}), 5000.0);
    // offline events carry no name: the one learned earlier is used
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": M1, "online": false}), 6000.0);
    s.apply_event(&json!({"event": "app", "project": PID, "error": "control port closed"}), 7000.0);
    s.apply_event(&json!({"event": "something-new"}), 8000.0);
    let lines: Vec<String> = s.activity.iter().rev().map(|a| a.said.text()).collect();
    assert_eq!(
        lines,
        [
            "Member1 is online",
            "Member1 changed photos",
            "You changed photos",
            "Your changes and Member1's were put together · 3 clashes",
            "Up to date with Member1",
            "Member1 went offline",
            "Lost touch with this window for a moment; it reconnects by itself",
        ]
    );
    assert_eq!(s.projects[0].head.as_deref(), Some("abcd"));
    assert_eq!(s.projects[0].teammates, [Teammate { device: M1.into(), account: None, name: Some("Member1".into()), online: false }]);
    // an unknown teammate is named by the short device id
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": NEW_PID, "online": true}), 9000.0);
    assert_eq!(s.activity[0].said.text(), "bb11bb22 is online");
    assert_eq!(Said::Merged { who: "Alice".into(), clashes: 1 }.text(), "Your changes and Alice's were put together · 1 clash");
    assert_eq!(Said::Merged { who: "Alice".into(), clashes: 0 }.text(), "Your changes and Alice's were put together");
}

#[test]
fn photo_counts_are_said_when_the_service_gives_them() {
    let mut s = state_with_project();
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward", "photos": 2}), 0.0);
    assert_eq!(s.activity[0].said.text(), "Member1 changed 2 photos");
    s.apply_event(&json!({"event": "saved", "project": PID, "photos": 1}), 0.0);
    assert_eq!(s.activity[0].said.text(), "You changed 1 photo");
    s.apply_event(&json!({"event": "saved", "project": PID, "photos": 0}), 0.0);
    assert_eq!(s.activity[0].said.text(), "You changed photos");
}

#[test]
fn the_same_thing_again_counts_instead_of_repeating() {
    let mut s = state_with_project();
    for t in 0..5 {
        s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward"}), f64::from(t) * 100.0);
    }
    assert_eq!(s.activity.len(), 1);
    assert_eq!((s.activity[0].times, s.activity[0].at_ms), (5, 400.0));
    for t in 0..(ACTIVITY_KEEP + 10) {
        s.note(None, Said::Changed { who: format!("m{t}"), photos: None }, 0.0);
    }
    assert_eq!(s.activity.len(), ACTIVITY_KEEP);
}

#[test]
fn time_ago_in_plain_words() {
    assert_eq!(ago(0.4), "just now");
    assert_eq!(ago(-5.0), "just now");
    assert_eq!(ago(f64::NAN), "just now");
    assert_eq!(ago(3.7), "3 s ago");
    assert_eq!(ago(125.0), "2 min ago");
    assert_eq!(ago(7300.0), "2 h ago");
    assert_eq!(ago(200000.0), "2 d ago");
}

#[test]
fn status_names_teammates_and_leaves_me_out() {
    let mut s = state_with_project();
    s.apply_status(PID, &json!({"head": "ff00", "teammates": [M1, ALICE], "online": {M1: "Member1"}}));
    assert_eq!(s.projects[0].teammates, [Teammate { device: M1.into(), account: None, name: Some("Member1".into()), online: true }]);
    // later the teammate is offline: the name stays
    s.apply_status(PID, &json!({"head": "ff00", "teammates": [M1], "online": {}}));
    assert_eq!(s.projects[0].teammates[0].label(), "Member1");
    assert!(!s.projects[0].teammates[0].online);
    // hello again keeps what status said
    s.apply_hello(&json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "projects": [{"id": PID, "name": "Shoot"}]}));
    assert_eq!(s.projects[0].head.as_deref(), Some("ff00"));
    assert_eq!(s.projects[0].teammates.len(), 1);
    // junk is ignored, never a panic
    s.apply_status(PID, &json!({"teammates": 5, "online": [1]}));
    s.apply_hello(&json!({"projects": [{"name": 3}, 7]}));
    s.apply_event(&json!({"event": "teammate"}), 0.0);
    s.apply_event(&json!(null), 0.0);
}

#[test]
fn ids_are_checked() {
    assert!(is_hex_id(M1));
    assert!(!is_hex_id(&M1[..63]));
    assert!(!is_hex_id(&format!("{}g", &M1[..63])));
    assert_eq!(short(M1), "2635f5f7");
    assert_eq!(short("ab"), "ab");
    assert_eq!(short("ééééééééé"), "ééééééééé".get(..8).unwrap_or("ééééééééé"));
}

// ---- a fake kopanod over loopback TCP ------------------------------------------------------

#[derive(Default)]
struct FakeData {
    /// The Light control port the project is bound to (0: none); `attach` changes it.
    app_port: AtomicUsize,
    signed_out: AtomicBool,
    /// The project is "I approve changes": this side owns it (pending work), or is the member.
    approve_owner: AtomicBool,
    /// With `approve_owner`: Member2's changes wait too, clashing on the same photo setting.
    two_members: AtomicBool,
    approve_member: AtomicBool,
    /// A live project with one open clash.
    live_clash: AtomicBool,
    /// Not signed in, a sign-in waits in the browser for code BCDF-1234.
    signing_in: AtomicBool,
    /// ThabangVision people on the project; this side is its owner unless `member_role`.
    people: AtomicBool,
    member_role: AtomicBool,
    /// "Shared with me" lists one library that is not on this computer.
    shared_rows: AtomicBool,
}

struct Fake {
    got: Arc<Mutex<Vec<Value>>>,
    data: Arc<FakeData>,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    /// Serve the API on `port` (0 = any; see `port()`), with `home/api.token`.
    fn start(listener: TcpListener) -> Self {
        listener.set_nonblocking(true).unwrap();
        let got: Arc<Mutex<Vec<Value>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let data: Arc<FakeData> = Arc::default();
        let (g, st, ap) = (got.clone(), stop.clone(), data.clone());
        let accept = std::thread::spawn(move || {
            let mut workers = vec![];
            while !st.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((c, _)) => {
                        let (g, st, ap) = (g.clone(), st.clone(), ap.clone());
                        workers.push(std::thread::spawn(move || serve(c, &g, &st, &ap)));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });
        Fake { got, data, stop, accept: Some(accept) }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(a) = self.accept.take() {
            a.join().unwrap();
        }
    }

    fn requests(&self, op: &str) -> Vec<Value> {
        self.got.lock().unwrap().iter().filter(|r| r["op"] == op).cloned().collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(c: TcpStream, got: &Mutex<Vec<Value>>, stop: &AtomicBool, data: &FakeData) {
    let app_port = &data.app_port;
    c.set_nonblocking(false).unwrap();
    c.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut w = c.try_clone().unwrap();
    let mut r = BufReader::new(c);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        match r.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => continue, // timeout: look at `stop` again (a partial line stays in `line`)
        }
        let req: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
        line.clear();
        if req["token"] != TOKEN {
            let _ = writeln!(w, "{}", json!({"ok": false, "error": "bad token"}));
            return;
        }
        got.lock().unwrap().push(req.clone());
        let result = match req["op"].as_str().unwrap_or_default() {
            "subscribe" => {
                let _ = writeln!(w, "{}", json!({"ok": true, "result": "subscribed"}));
                for ev in [
                    json!({"event": "teammate", "project": PID, "device": M1, "name": "Member1", "online": true}),
                    json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward", "clashes": 0, "photos": 2, "head": "1234"}),
                ] {
                    let _ = writeln!(w, "{ev}");
                }
                while !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                return;
            }
            "hello" => {
                let waiting = data.signing_in.load(Ordering::SeqCst);
                let out = data.signed_out.load(Ordering::SeqCst) || waiting;
                json!({"device": ALICE, "name": if out { "" } else { "Alice" }, "account": if out { "" } else { "acct-alice" }, "signed_in": !out,
                    "signed_in_until_ms": 1.8e12,
                    "signing_in": if waiting { json!({"code": "BCDF-1234", "link": "http://localhost:3100/link?code=BCDF-1234", "until_ms": 1.8e12}) } else { Value::Null },
                    "message": if out && !waiting { json!("This computer was unlinked from ThabangVision. Sign in again.") } else { Value::Null },
                    "projects": if out { json!([]) } else { json!([{"id": PID, "name": "Shoot", "app": app(app_port)}]) }})
            }
            "people" if data.people.load(Ordering::SeqCst) => {
                let owner = !data.member_role.load(Ordering::SeqCst);
                json!({"shared": "tv-1", "people": [
                    {"user": "u-alice", "name": "Alice", "role": if owner { "owner" } else { "editor" }, "status": "active", "me": true, "online": true, "on_project": true},
                    {"user": "u-bongi", "name": "Bongi", "role": if owner { "editor" } else { "owner" }, "status": "active", "me": false, "online": true, "on_project": true},
                    {"user": "u-sipho", "name": "Sipho", "role": "editor", "status": "invited", "me": false, "online": false, "on_project": false}]})
            }
            "people" => json!({"shared": false, "people": []}),
            "shared-projects" => {
                let rows = if data.shared_rows.load(Ordering::SeqCst) {
                    json!([{"shared": "tv-shared-77", "name": "Lindiwe's shoot", "project": NEW_PID, "role": "editor", "status": "invited", "here": false}])
                } else {
                    json!([])
                };
                json!({"mine": [], "shared": rows})
            }
            "sign-in" => json!({"code": "BCDF-1234", "link": "http://localhost:3100/link?code=BCDF-1234", "until_ms": 1.8e12}),
            "sign-in-cancel" => json!("not signing in"),
            "sign-out" => json!("signed out"),
            "invite" => json!({"message": "Invite sent."}),
            "remove-person" => json!({"removed": true}),
            "leave" => json!({"left": true}),
            "join-shared" => json!({"project": NEW_PID, "name": "Lindiwe's shoot"}),
            "status" => {
                let mut st = json!({"head": "1234abcd", "name": "Shoot", "app": app(app_port),
                "teammates": [{"device": M1, "name": "Member1", "account": "acct-m1", "online": true}], "online": {M1: "Member1"},
                "members": [{"account": "acct-alice", "role": "editor", "owner": true}, {"account": "acct-m1", "role": "editor", "owner": false}]});
                if data.approve_owner.load(Ordering::SeqCst) {
                    st["mode"] = json!("approve");
                    st["review"] = json!({"is_owner": true, "approver": true, "owner_name": "Alice", "declined": [], "mine": null, "viewing": null,
                        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 3, "summary": "changed 3 photos",
                            "changes": [{"photo": "abcdef0123", "fields": []}],
                            "clashes": [{"key": "abcdef0123#develop", "photo": "abcdef0123", "fields": ["Light exposure"],
                                "settings": [{"setting": "light.exposure", "main": -1.0, "member": 1.5}]}]}]});
                    if data.two_members.load(Ordering::SeqCst) {
                        let mut m2 = st["review"]["pending"][0].clone();
                        m2["account"] = json!("acct-m2");
                        m2["name"] = json!("Member2");
                        m2["version"] = json!("v2");
                        st["review"]["pending"].as_array_mut().unwrap().push(m2);
                    }
                } else if data.approve_member.load(Ordering::SeqCst) {
                    st["mode"] = json!("approve");
                    st["review"] = json!({"is_owner": false, "approver": false, "owner_name": "Alice", "declined": [], "pending": [], "viewing": null,
                        "mine": {"state": "waiting", "photos": 2}});
                } else if data.live_clash.load(Ordering::SeqCst) {
                    st["mode"] = json!("live");
                    st["clashes"] = json!([{"clash": "kopano/conflict/k/11-22", "key": "k#develop", "photo": "k", "fields": ["Light exposure"],
                        "settings": [{"setting": "light.exposure", "a": 0.5, "b": -0.25}]}]);
                }
                st
            }
            "approve" => json!({"main": "m2", "clashes_decided": 1}),
            "decline" => json!({"declined": "v1"}),
            "view" => json!({"viewing": {"side": req["show"]}}),
            "resolve" => json!({"settled": req["clash"]}),
            "set-approver" => json!({"approver": true}),
            "create-project" => json!({"project": NEW_PID, "head": "99", "items": 15, "photos": 5}),
            "attach" => {
                app_port.store(req["app"]["port"].as_u64().unwrap_or(0) as usize, Ordering::SeqCst);
                json!("attached")
            }
            "join" => json!({"project": req["project"]}),
            _ => {
                let _ = writeln!(w, "{}", json!({"ok": false, "error": "unknown op"}));
                continue;
            }
        };
        let _ = writeln!(w, "{}", json!({"ok": true, "result": result}));
    }
}

fn app(port: &AtomicUsize) -> Value {
    match port.load(Ordering::SeqCst) {
        0 => json!({"kind": "toy"}),
        p => json!({"kind": "light-control", "port": p}),
    }
}

fn wait(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while !done() {
        assert!(t0.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fake_service() -> (Fake, Config, PathBuf) {
    let home = temp_home();
    std::fs::write(home.join("api.token"), format!("{TOKEN}\n")).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (Fake::start(listener), Config { port, home: Some(home.clone()) }, home)
}

const T: Duration = Duration::from_secs(20);

#[test]
fn client_follows_a_fake_service_and_finds_it_again_after_a_restart() {
    let (mut fake, config, _home) = fake_service();
    let mut client = Client::new(config.clone());
    client.start(&egui::Context::default());
    wait("hello and status", T, || {
        let s = client.state();
        s.running && s.me.as_ref().is_some_and(|m| m.name == "Alice") && s.projects.first().is_some_and(|p| p.teammates.iter().any(|t| t.online))
    });
    wait("events", T, || client.state().activity.iter().any(|a| a.said.text() == "Member1 changed 2 photos"));
    let s = client.state();
    assert_eq!(s.projects[0].name, "Shoot");
    assert_eq!(s.projects[0].teammates[0].label(), "Member1");

    // Share: the request carries the token, the app binding and the teammate
    client
        .request(
            &egui::Context::default(),
            "Sharing…",
            json!({"op": "create-project", "name": "Shoot 2", "app": {"kind": "light-control", "port": 18123}, "members": [{"account": "acct-m1", "role": "editor", "lanes": []}]}),
        )
        .unwrap();
    wait("the share", T, || client.state().job.is_some_and(|j| !j.busy));
    let job = client.state().job.unwrap();
    assert!(job.ok, "{job:?}");
    assert_eq!(job.project.as_deref(), Some(NEW_PID));
    assert!(job.text.starts_with("Shared 5 photos. Invite sent."), "{}", job.text);
    let made = fake.requests("create-project");
    assert_eq!(made.len(), 1);
    assert_eq!(made[0]["app"], json!({"kind": "light-control", "port": 18123}));
    assert_eq!(made[0]["token"], TOKEN);

    // the service stops: the panel says so and teammates go grey
    fake.stop();
    // (a poll under way when it stopped may first say "stopped answering")
    wait("not running", T, || client.state().problem.as_deref() == Some(NOT_RUNNING));
    let s = client.state();
    assert!(!s.running);
    assert!(s.projects[0].teammates.iter().all(|t| !t.online));
    // and it comes back on the same port: found again with no help
    let listener = TcpListener::bind(("127.0.0.1", config.port)).unwrap();
    let fake2 = Fake::start(listener);
    wait("running again", T, || client.state().running);
    wait("events again", T, || !fake2.requests("subscribe").is_empty());
}

#[test]
fn a_wrong_token_is_explained() {
    let (_fake, config, home) = fake_service();
    std::fs::write(home.join("api.token"), "other").unwrap();
    let mut client = Client::new(config);
    client.start(&egui::Context::default());
    wait("the refusal", T, || client.state().problem.is_some());
    let s = client.state();
    assert!(!s.running);
    assert!(s.problem.unwrap().contains("api.token"));
}

// ---- the panel in the app, driven through the control channel -----------------------------

fn app_with(config: Config) -> crate::headless::Headless {
    let mut app = crate::LightcraftApp::new(lightcraft_engine::Session::with_demo(), Default::default());
    app.kopano = KopanoPanel::with_config(config);
    let mut h = crate::headless::Headless::new(app, [1400.0, 900.0], 1.0);
    h.settle(Duration::from_secs(120));
    h
}

fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}

fn painted(h: &crate::headless::Headless) -> String {
    h.painted_text().join("\n")
}

fn type_into(h: &mut crate::headless::Headless, widget: &str, text: &str) {
    ok(h.request("ui.clickWidget", json!({"id": widget}), T));
    ok(h.request("ui.text", json!({"text": text}), T));
}

/// End users see ThabangVision collaboration: never the protocol's name.
fn no_protocol_words(text: &str) {
    for word in ["Kopano", "kopano", "protocol"] {
        assert!(!text.contains(word), "{word:?} on screen: {text}");
    }
}

#[test]
fn window_says_when_collaboration_is_not_running() {
    // signed in (a certificate), but nothing listens on the port
    let home = temp_home();
    std::fs::write(home.join("cert.json"), "{}").unwrap();
    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let mut h = app_with(Config { port, home: Some(home) });
    assert_eq!(ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T)), json!({"open": true, "advanced": false}));
    assert!(h.step_until(T, |h| painted(h).contains(NOT_RUNNING)), "{}", painted(&h));
    assert!(painted(&h).contains("Start ThabangVision collaboration on this computer"));
    assert!(painted(&h).contains("Share & Collaborate"));
    no_protocol_words(&painted(&h));
    // Join is under Advanced (testing): closed, it isn't on screen
    assert!(!painted(&h).contains("Teammate's device id"));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-advanced"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Teammate's device id")), "{}", painted(&h));
    // the section opens with an animation: let it finish before clicking inside
    for _ in 0..40 {
        h.step();
    }
    // Join checks its fields before anything else
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-join"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("The project id is 64 letters")), "{}", painted(&h));
    // Window menu lists it; the command closes it again
    let menus = ok(h.request("ui.menu.list", json!({}), T));
    assert!(
        menus.as_array().unwrap().iter().any(|m| m["id"] == "kopano.panel" && m["menu"][0] == "Window" && m["label"] == "Share & Collaborate"),
        "{menus}"
    );
    assert_eq!(ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T))["open"], false);
    h.step();
    assert!(!painted(&h).contains(NOT_RUNNING));
    // the toolbar button opens it too
    ok(h.request("ui.clickWidget", json!({"id": "icon:collaborate"}), T));
    assert!(h.step_until(T, |h| painted(h).contains(NOT_RUNNING)), "{}", painted(&h));
}

#[test]
fn without_a_sign_in_the_window_offers_one() {
    // the service runs, but this computer is not signed in to ThabangVision (or it ran out)
    let (fake, config, _home) = fake_service();
    fake.data.signed_out.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Sign in with ThabangVision")), "{}", painted(&h));
    // nothing else shows until signed in; the service's sentence says why
    assert!(painted(&h).contains("unlinked from ThabangVision") && !painted(&h).contains("Share this library"), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-signIn"}), T));
    wait("the sign-in", T, || fake.requests("sign-in").len() == 1);
    assert!(h.step_until(T, |h| painted(h).contains("Check that the browser shows BCDF-1234.")), "{}", painted(&h));
    no_protocol_words(&painted(&h));
}

#[test]
fn window_shares_through_its_fields_and_starts_the_control_port_on_demand() {
    let (fake, config, _home) = fake_service();
    let mut h = app_with(config);
    let started = Arc::new(AtomicUsize::new(0));
    let s2 = started.clone();
    h.app.kopano.start_control = Some(Box::new(move || {
        s2.fetch_add(1, Ordering::SeqCst);
        Ok((18999, None))
    }));
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(
        h.step_until(T, |h| {
            let p = painted(h);
            p.contains("Signed in to ThabangVision as Alice")
                && p.contains("Member1")
                && p.contains("online")
                && p.contains("Member1 changed 2 photos")
        }),
        "{}",
        painted(&h)
    );
    // no ids and no protocol name while Advanced is closed
    let p = painted(&h);
    no_protocol_words(&p);
    assert!(!p.contains("c013b977") && !p.contains(&PID[..8]), "{p}");
    ok(h.request("engine.execute", json!({"command": "kopano.panel", "params": {"open": true, "advanced": true}}), T));
    assert!(h.step_until(T, |h| painted(h).contains("c013b977")), "{}", painted(&h));

    // Copy puts the whole device id on the clipboard
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-copyDevice"}), T));
    h.step();
    assert_eq!(h.app.copied.as_deref(), Some(ALICE));

    type_into(&mut h, "kopano:shareName", "Owner shoot");
    type_into(&mut h, "kopano:shareAccount", "acct-m1");
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-share"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Shared 5 photos. Invite sent.")), "{}", painted(&h));
    let made = fake.requests("create-project");
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0]["name"], "Owner shoot");
    assert_eq!(made[0]["app"], json!({"kind": "light-control", "port": 18999}));
    assert_eq!(made[0]["members"], json!([{"account": "acct-m1", "role": "editor", "lanes": []}]));
    // Join uses the same control port: it was started once
    ok(h.request("engine.execute", json!({"command": "kopano.join", "params": {"project": PID, "from": M1}}), T));
    wait("the join", T, || fake.requests("join").len() == 1);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(fake.requests("join")[0]["app"]["port"], 18999);
    let status = ok(h.request("engine.execute", json!({"command": "kopano.status"}), T));
    assert_eq!(status["controlPort"], 18999);
    assert_eq!(status["running"], true);
}

#[test]
fn share_without_a_control_port_says_how_to_get_one() {
    let (_fake, config, _home) = fake_service();
    let mut h = app_with(config);
    let r = h.request("engine.execute", json!({"command": "kopano.share", "params": {"name": "x", "account": "acct-m1"}}), T);
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("can't be shared right now"), "{r}");
    let r = h.request("engine.execute", json!({"command": "kopano.share", "params": {"name": "", "account": "acct-m1"}}), T);
    assert!(r["error"].as_str().unwrap().contains("name"), "{r}");
}

#[test]
fn new_status_keeps_names_accounts_members_and_the_app_port() {
    let mut s = State::default();
    s.apply_hello(&json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "signed_in": true, "signed_in_until_ms": 1.5e12,
        "projects": [{"id": PID, "name": "Shoot", "app": {"kind": "light-control", "port": 18101}}]}));
    assert!(s.signed_in);
    assert_eq!(s.signed_in_until_ms, Some(1.5e12));
    assert_eq!(s.projects[0].app_port, Some(18101));
    // an offline teammate keeps the name the service gives
    s.apply_status(
        PID,
        &json!({"head": "ab", "name": "Shoot", "app": {"kind": "light-control", "port": 18101},
        "teammates": [{"device": M1, "name": "Member1", "account": "acct-m1", "online": false}], "online": {},
        "members": [{"account": "acct-alice", "role": "editor", "owner": true}]}),
    );
    let t = &s.projects[0].teammates[0];
    assert_eq!((t.label(), t.account.as_deref(), t.online), ("Member1".to_string(), Some("acct-m1"), false));
    assert_eq!(s.projects[0].members, [Member { account: "acct-alice".into(), role: "editor".into(), owner: true }]);
    // status moves the port (after an attach): its liveness is unknown again
    s.projects[0].app_alive = Some(false);
    s.apply_status(PID, &json!({"app": {"kind": "light-control", "port": 18555}, "teammates": []}));
    assert_eq!((s.projects[0].app_port, s.projects[0].app_alive), (Some(18555), None));
    assert_eq!(app_port(Some(&json!({"kind": "toy"}))), None);
    assert_eq!(app_port(Some(&json!({"kind": "light-control", "port": 70000}))), None);
}

#[test]
fn records_say_which_libraries_this_library_holds() {
    let home = temp_home();
    let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<std::collections::BTreeSet<_>>();
    assert!(read_record(&home, PID).is_empty());
    write_record(&home, PID, &set(&["a", "b", "c", "d"])).unwrap();
    assert_eq!(read_record(&home, PID), set(&["a", "b", "c", "d"]));
    assert!(holds(&read_record(&home, PID), &set(&["a", "b", "x"])));
    assert!(!holds(&read_record(&home, PID), &set(&["a", "x"])));
    assert!(!holds(&set(&[]), &set(&["a"])));
    // a bad id never becomes a path
    assert!(write_record(&home, "../evil", &set(&["a"])).is_err());
    std::fs::write(record_path(&home, PID).unwrap(), "junk").unwrap();
    assert!(read_record(&home, PID).is_empty());
}

fn our_keys(h: &crate::headless::Headless) -> std::collections::BTreeSet<String> {
    h.app
        .session
        .catalog
        .photos()
        .filter(|p| !p.deleted && !p.local && p.copy_of.is_none())
        .map(|p| p.content_hash.clone().unwrap_or_else(|| format!("{}|{}", p.file_name, p.captured.as_deref().unwrap_or_default())))
        .collect()
}

#[test]
fn after_a_restart_the_window_links_its_shared_library_again() {
    let (fake, config, home) = fake_service();
    // the library was bound to a port nobody answers on any more (Light restarted)
    let dead = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    fake.data.app_port.store(usize::from(dead), Ordering::SeqCst);
    let mut h = app_with(config);
    h.app.kopano.control_port = Some(18777);
    // this library holds the shared library's photos
    let ours = our_keys(&h);
    assert!(!ours.is_empty());
    write_record(&home, PID, &ours).unwrap();
    // with the window closed: the background work does it
    assert!(h.step_until(T, |_| fake.data.app_port.load(Ordering::SeqCst) == 18777), "attach not sent: {:?}", fake.requests("attach"));
    assert_eq!(fake.requests("attach")[0]["app"], json!({"kind": "light-control", "port": 18777}));
    assert!(h.step_until(T, |h| h.app.kopano.client.state().activity.iter().any(|a| a.said.text() == "This window is linked to Shoot again")));
    assert_eq!(read_record(&home, PID), ours);
    for _ in 0..50 {
        h.step();
    }
    assert_eq!(fake.requests("attach").len(), 1, "attached once");
}

#[test]
fn a_dead_port_of_a_library_this_one_does_not_hold_is_left_alone() {
    let (fake, config, home) = fake_service();
    let dead = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    fake.data.app_port.store(usize::from(dead), Ordering::SeqCst);
    write_record(&home, PID, &["someone-else|2020".to_string()].into_iter().collect()).unwrap();
    let mut h = app_with(config);
    h.app.kopano.control_port = Some(18778);
    assert!(h.step_until(T, |h| h.app.kopano.client.state().projects.first().is_some_and(|p| p.app_alive == Some(false))));
    for _ in 0..200 {
        h.step();
    }
    assert!(fake.requests("attach").is_empty());
}

#[test]
fn errors_about_the_old_port_right_after_relinking_are_not_shown() {
    let mut s = state_with_project();
    s.note(Some(PID.into()), Said::Reattached { name: "Shoot".into() }, 1000.0);
    s.apply_event(&json!({"event": "app", "project": PID, "error": "port 18101 refused"}), 5000.0);
    assert_eq!(s.activity.len(), 1);
    // later trouble shows again
    s.apply_event(&json!({"event": "app", "project": PID, "error": "port 18103 refused"}), 1000.0 + RELINK_QUIET_MS + 1.0);
    assert_eq!(s.activity[0].said.text(), "Lost touch with this window for a moment; it reconnects by itself");
}

#[test]
fn review_lines_read_in_plain_words() {
    let owner = Review::from_value(&json!({"is_owner": true, "approver": true, "owner_name": "Alice",
        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 3, "changes": [{"photo": "a"}, {"photo": "b"}],
            "clashes": [{"key": "a#develop", "photo": "a", "settings": [{"setting": "light.exposure", "main": -1.0, "member": 1.5}]}]}]}));
    assert_eq!(owner.pending[0].name, "Member1");
    assert_eq!(owner.pending[0].changed, ["a", "b"]);
    assert_eq!(owner.pending[0].clashes[0].main, "-1");
    assert_eq!(owner.pending[0].clashes[0].member, "1.50");
    assert_eq!(setting_text("light.exposure"), "Exposure");
    assert_eq!(owner.mine_text(), None);
    let mine = |state: &str| Review::from_value(&json!({"owner_name": "Alice", "mine": {"state": state, "photos": 2}})).mine_text().unwrap();
    assert_eq!(mine("waiting"), "You changed 2 photos · Waiting for Alice's approval");
    assert!(mine("approved").starts_with("Approved"));
    assert!(mine("declined").starts_with("Declined"));
    // events: new work to review, and what happened to mine
    let mut s = state_with_project();
    s.apply_event(
        &json!({"event": "review", "project": PID, "is_owner": true, "approver": true, "owner_name": "Alice",
        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 3}]}),
        1000.0,
    );
    assert_eq!(s.activity[0].said.text(), "Member1 changed 3 photos · waiting for your review");
    let mut m = state_with_project();
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "waiting", "photos": 1}}), 1000.0);
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "approved", "photos": 0}}), 2000.0);
    assert_eq!(m.activity[0].said.text(), "Alice approved your changes");
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "declined", "photos": 1}}), 3000.0);
    assert_eq!(m.activity[0].said.text(), "Alice declined your changes");
    no_protocol_words(&m.activity.iter().map(|a| a.said.text()).collect::<Vec<_>>().join("\n"));
}

#[test]
fn the_owner_reviews_views_approves_and_declines_from_the_window() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_owner.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    h.app.kopano.start_control = Some(Box::new(|| Ok((18999, None))));
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(
        h.step_until(T, |h| {
            let p = painted(h);
            p.contains("Changes to review") && p.contains("Member1 changed 3 photos") && p.contains("You approve changes")
        }),
        "{}",
        painted(&h)
    );
    let p = painted(&h);
    no_protocol_words(&p);
    assert!(p.contains("Exposure") && p.contains("Keep yours") && p.contains("Take theirs"), "{p}");
    let tag = short(PID);
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-view-{tag}-acct-m1")}), T));
    wait("the view", T, || fake.requests("view").len() == 1);
    assert_eq!(fake.requests("view")[0]["show"], "member");
    assert_eq!(fake.requests("view")[0]["member"], "acct-m1");
    for _ in 0..5 {
        h.step();
    }
    // the clash: keep main's value for that photo, then approve with that pick
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-keepMain-{tag}-abcdef01-acct-m1")}), T));
    h.step();
    wait("the job", T, || !h.app.kopano.client.state().job.as_ref().is_some_and(|j| j.busy));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-approve-{tag}-acct-m1")}), T));
    wait("the approve", T, || fake.requests("approve").len() == 1);
    let a = &fake.requests("approve")[0];
    assert_eq!(a["member"], "acct-m1");
    assert_eq!(a["version"], "v1");
    assert_eq!(a["choices"], json!({"abcdef0123#develop": "main"}));
    assert!(h.step_until(T, |h| painted(h).contains("Approved. The changes are in your library.")), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-decline-{tag}-acct-m1")}), T));
    wait("the decline", T, || fake.requests("decline").len() == 1);
    assert!(h.step_until(T, |h| painted(h).contains("Declined. Your library stays as it was.")), "{}", painted(&h));
    // Share asks how to work: "I approve changes" unless live is picked
    type_into(&mut h, "kopano:shareName", "Owner shoot");
    type_into(&mut h, "kopano:shareAccount", "acct-m1");
    assert!(painted(&h).contains("I approve changes") && painted(&h).contains("Work together live"));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-share"}), T));
    wait("the share", T, || fake.requests("create-project").len() == 1);
    assert_eq!(fake.requests("create-project")[0]["mode"], "approve");
    wait("the job", T, || !h.app.kopano.client.state().job.as_ref().is_some_and(|j| j.busy));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-modeLive"}), T));
    h.step();
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-share"}), T));
    wait("the second share", T, || fake.requests("create-project").len() == 2);
    assert_eq!(fake.requests("create-project")[1]["mode"], "live");
}

#[test]
fn a_member_sees_where_his_changes_are() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_member.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Waiting for Alice's approval")), "{}", painted(&h));
    let p = painted(&h);
    assert!(p.contains("Alice approves changes") && !p.contains("Changes to review"), "{p}");
    no_protocol_words(&p);
}

#[test]
fn a_live_clash_offers_both_values() {
    let (fake, config, _home) = fake_service();
    fake.data.live_clash.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Pick the one to keep")), "{}", painted(&h));
    let p = painted(&h);
    assert!(p.contains("0.50") && p.contains("-0.25") && p.contains("Works live"), "{p}");
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-pickB-{}-0", short(PID))}), T));
    wait("the resolve", T, || fake.requests("resolve").len() == 1);
    assert_eq!(fake.requests("resolve")[0]["pick"], "b");
    assert_eq!(fake.requests("resolve")[0]["clash"], "kopano/conflict/k/11-22");
    no_protocol_words(&painted(&h));
}

/// Review finding 5: two members changed the same setting. Each member's clash picks are their
/// own: approving Member1 sends only Member1's picks and keeps Member2's for later.
#[test]
fn clash_picks_belong_to_one_member() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_owner.store(true, Ordering::SeqCst);
    fake.data.two_members.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    h.app.kopano.start_control = Some(Box::new(|| Ok((18999, None))));
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Member2 changed 3 photos")), "{}", painted(&h));
    let tag = short(PID);
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-keepMain-{tag}-abcdef01-acct-m1")}), T));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-takeMember-{tag}-abcdef01-acct-m2")}), T));
    let choose = |member: &str, key: &str, pick: &str| json!({"command": "kopano.choose", "params": {"project": PID, "member": member, "key": key, "pick": pick}});
    ok(h.request("engine.execute", choose("acct-m2", "abcdef0123#develop", "main"), T));
    ok(h.request("engine.execute", choose("acct-m2", "other#develop", "member"), T));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-approve-{tag}-acct-m1")}), T));
    wait("the approve", T, || fake.requests("approve").len() == 1);
    assert_eq!(fake.requests("approve")[0]["choices"], json!({"abcdef0123#develop": "main"}), "only Member1's picks");
    let left: Vec<_> = h.app.kopano.choices.keys().cloned().collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.iter().all(|(_, who, _)| who == "acct-m2"), "{left:?}");
    wait("the job", T, || !h.app.kopano.client.state().job.as_ref().is_some_and(|j| j.busy));
    ok(h.request("engine.execute", json!({"command": "kopano.decline", "params": {"project": PID, "member": "acct-m2", "version": "v2"}}), T));
    assert!(h.app.kopano.choices.is_empty());
    let r = h.request("engine.execute", json!({"command": "kopano.choose", "params": {"project": PID, "key": "x", "pick": "main"}}), T);
    assert_eq!(r["ok"], false, "a pick without the member is refused");
}

/// Review finding 7: problems are said in plain words, never with a file path, the service's
/// words or the protocol's name.
#[test]
fn problems_are_said_in_plain_words() {
    for raw in ["the collaboration key file is missing (C:\\Users\\x\\AppData\\Local\\Kopano\\api.token)", "bad token", "unknown op create-project"] {
        let said = plain(raw);
        assert!(!said.contains('\\') && !said.contains('/') && !said.to_lowercase().contains("kopano") && !said.contains("op "), "{raw} -> {said}");
    }
    assert_eq!(plain("bad token"), "The collaboration service is not running. Restart Light.");
    assert_eq!(plain("Give the shared library a name"), "Give the shared library a name");
}

/// Sign in with ThabangVision: the code to check in the browser, Cancel, and Sign out.
#[test]
fn signing_in_shows_the_code_and_can_be_stopped_and_signed_out() {
    let (fake, config, _home) = fake_service();
    fake.data.signing_in.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Check that the browser shows BCDF-1234")), "{}", painted(&h));
    assert!(!painted(&h).contains("Shared libraries"), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-signInCancel"}), T));
    wait("the cancel", T, || fake.requests("sign-in-cancel").len() == 1);
    assert!(h.step_until(T, |h| painted(h).contains("Sign-in stopped.")), "{}", painted(&h));
    fake.data.signing_in.store(false, Ordering::SeqCst);
    assert!(h.step_until(T, |h| painted(h).contains("Signed in to ThabangVision as Alice")), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-signOut"}), T));
    wait("the sign-out", T, || fake.requests("sign-out").len() == 1);
    assert!(h.step_until(T, |h| painted(h).contains("Signed out of ThabangVision on this computer.")), "{}", painted(&h));
    no_protocol_words(&painted(&h));
}

/// Share by email: the library is made, then each email is invited ("Invite sent"). Live mode
/// cannot take invites, so it is refused up front in plain words.
#[test]
fn share_by_email_invites_each_teammate() {
    let (fake, config, _home) = fake_service();
    let mut h = app_with(config);
    h.app.kopano.start_control = Some(Box::new(|| Ok((18999, None))));
    let share = |email: &str, mode: &str| json!({"command": "kopano.share", "params": {"name": "Shoot", "email": email, "mode": mode}});
    let r = h.request("engine.execute", share("bongi@example.com", "live"), T);
    assert!(r["error"].as_str().unwrap().contains("I approve changes"), "{r}");
    ok(h.request("engine.execute", share("bongi@example.com, sipho@example.com", "approve"), T));
    wait("the invites", T, || fake.requests("invite").len() == 2);
    assert_eq!(fake.requests("create-project")[0]["members"], json!([]), "emails are invited, not made members");
    let inv = fake.requests("invite");
    assert_eq!((inv[0]["project"].as_str(), inv[0]["email"].as_str()), (Some(NEW_PID), Some("bongi@example.com")));
    assert_eq!(inv[1]["email"], "sipho@example.com");
}

/// The people on a shared library by name and role (never emails); the owner invites and takes
/// people off; a member may leave.
#[test]
fn people_by_name_with_invite_remove_and_leave() {
    let (fake, config, _home) = fake_service();
    fake.data.people.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Bongi")), "{}", painted(&h));
    let p = painted(&h);
    assert!(p.contains("Sipho") && p.contains("invited") && p.contains("can edit") && !p.contains('@'), "{p}");
    let tag = short(PID);
    type_into(&mut h, &format!("kopano:invite-{tag}"), "thandi@example.com");
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-invite-{tag}")}), T));
    wait("the invite", T, || fake.requests("invite").len() == 1);
    assert_eq!(fake.requests("invite")[0]["email"], "thandi@example.com");
    assert!(h.step_until(T, |h| painted(h).contains("Invite sent.")), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-remove-{tag}-u-bongi")}), T));
    wait("the removal", T, || fake.requests("remove-person").len() == 1);
    assert_eq!(fake.requests("remove-person")[0]["user"], "u-bongi");
    fake.data.member_role.store(true, Ordering::SeqCst);
    wait("the job", T, || !h.app.kopano.client.state().job.as_ref().is_some_and(|j| j.busy));
    assert!(h.step_until(T, |h| painted(h).contains("Leave this library")), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-leave-{tag}")}), T));
    wait("the leave", T, || fake.requests("leave").len() == 1);
    assert!(h.step_until(T, |h| painted(h).contains("You left the library.")), "{}", painted(&h));
}

/// "Shared with me": Open here brings it in with nothing typed.
#[test]
fn shared_with_me_opens_here() {
    let (fake, config, _home) = fake_service();
    fake.data.shared_rows.store(true, Ordering::SeqCst);
    let mut h = app_with(config);
    h.app.kopano.start_control = Some(Box::new(|| Ok((18999, None))));
    ok(h.request("ui.menu.invoke", json!({"command": "kopano.panel"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Lindiwe's shoot")), "{}", painted(&h));
    ok(h.request("ui.clickWidget", json!({"id": format!("button:kopano-openShared-{}", short("tv-shared-77"))}), T));
    wait("the open", T, || fake.requests("join-shared").len() == 1);
    let r = &fake.requests("join-shared")[0];
    assert_eq!(r["shared"], "tv-shared-77");
    assert_eq!(r["app"], json!({"kind": "light-control", "port": 18999}));
}
