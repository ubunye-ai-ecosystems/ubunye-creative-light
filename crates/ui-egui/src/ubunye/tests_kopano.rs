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
                json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "signed_in": !data.signed_out.load(Ordering::SeqCst), "signed_in_until_ms": 1.8e12,
                "projects": [{"id": PID, "name": "Shoot", "app": app(app_port)}]})
            }
            "status" => json!({"head": "1234abcd", "name": "Shoot", "app": app(app_port),
                "teammates": [{"device": M1, "name": "Member1", "account": "acct-m1", "online": true}], "online": {M1: "Member1"},
                "members": [{"account": "acct-alice", "role": "editor", "owner": true}, {"account": "acct-m1", "role": "editor", "owner": false}]}),
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
    assert!(job.text.starts_with("Shared 5 photos. Until invites"), "{}", job.text);
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
    ok(h.request("ui.clickWidget", json!({"id": "button:kopano-signIn"}), T));
    assert!(h.step_until(T, |h| painted(h).contains("Sign-in is coming soon.")), "{}", painted(&h));
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
    assert!(h.step_until(T, |h| painted(h).contains("Shared 5 photos. Until invites")), "{}", painted(&h));
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
    assert!(r["error"].as_str().unwrap().contains("--control"), "{r}");
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
