//! The Share & Collaborate window (Window ▸ Share & Collaborate, and its toolbar button):
//! ThabangVision collaboration for this library. Ubunye, Kopano slice S3b.
//!
//! People see ThabangVision collaboration only: who they are signed in as, the shared libraries
//! with their teammates by name (green dot when online), recent activity in plain words, and
//! Share. No protocol names, device ids or project ids outside the collapsed "Advanced (testing)"
//! section, which keeps the id-based Share / Join until ThabangVision invites by username replace
//! it. The code keeps the protocol's name (Kopano) internally.
//!
//! Everything is a command: `kopano.panel` shows or hides the window, `kopano.share` and
//! `kopano.join` are what its buttons run, `kopano.signIn` is the sign-in button,
//! `kopano.status` is what it shows (for agents and tests).
//!
//! Two ways to work, chosen when sharing: "Work together live" (every edit goes to everyone) and
//! "I approve changes" (the default: teammates' edits wait in "Changes to review" until the
//! owner approves them; `kopano.approve`, `kopano.decline`, `kopano.view`, `kopano.choose`,
//! `kopano.setApprover`). Live clashes get a pick (`kopano.resolve`).
//!
//! The service reaches this window through Light's control port. With `--control PORT` that is
//! the port; without it, the first Share or Join starts the control server on demand
//! ([`StartControl`], supplied by the desktop app) and hands its port to the service.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::Receiver;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, vec2};
use serde_json::{Value, json};

use super::kopano::{self, Client, Config, Job, State};
use crate::LightcraftApp;
use crate::control::ControlRequest;
use crate::text_field::TextField;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

/// Start the control server if it isn't running: its port, and its request channel when this
/// call started it (the app then serves it like one started with `--control`).
pub type StartControl = Box<dyn FnMut() -> Result<(u16, Option<Receiver<ControlRequest>>), String>>;

/// The window's title, menu item and toolbar tooltip.
pub const TITLE: &str = "Share & Collaborate";
/// Online teammates' dot.
const GREEN: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x6b);
/// Activity lines shown (the rest are in `kopano.status`).
const SHOWN: usize = 6;

pub struct KopanoPanel {
    pub open: bool,
    /// The "Advanced (testing)" section is open.
    pub advanced: bool,
    pub client: Client,
    /// From the desktop app; none on the web and in tests that don't give one.
    pub start_control: Option<StartControl>,
    /// The control port the service was given (once known).
    pub control_port: Option<u16>,
    pub share_name: String,
    pub share_account: String,
    /// "approve" (default) or "live": how the next Share works.
    pub share_mode: String,
    /// The owner's pick per clash before approving: (project, member, item key) -> "member" |
    /// "main". Two members may change the same setting; each one's picks are their own.
    pub choices: BTreeMap<(String, String, String), String>,
    pub join_project: String,
    pub join_from: String,
    /// Shared libraries an `attach` was sent for (once each).
    pub attached: BTreeSet<String>,
    /// When the background work last ran, and this library's photos as last listed (wall ms).
    last_tick: f64,
    ours: (f64, BTreeSet<String>),
    /// What was last written to each owned library's record.
    recorded: BTreeMap<String, BTreeSet<String>>,
}

impl Default for KopanoPanel {
    fn default() -> Self {
        KopanoPanel::with_config(Config::from_env())
    }
}

impl KopanoPanel {
    pub fn with_config(config: Config) -> Self {
        KopanoPanel {
            open: false,
            advanced: false,
            client: Client::new(config),
            start_control: None,
            control_port: None,
            share_name: String::new(),
            share_account: String::new(),
            share_mode: "approve".to_string(),
            choices: BTreeMap::new(),
            join_project: String::new(),
            join_from: String::new(),
            attached: BTreeSet::new(),
            last_tick: f64::NEG_INFINITY,
            ours: (f64::NEG_INFINITY, BTreeSet::new()),
            recorded: BTreeMap::new(),
        }
    }
}

fn app_ctx(app: &LightcraftApp) -> egui::Context {
    app.tasks.repaint.clone().unwrap_or_default()
}

/// The port the service reaches this window on, starting the control server if needed.
fn control_port(app: &mut LightcraftApp) -> Result<u16, String> {
    if let Some(p) = app.kopano.control_port {
        return Ok(p);
    }
    let Some(start) = app.kopano.start_control.as_mut() else {
        return Err("This window can't be shared right now. Restart Light and try again.".to_string());
    };
    let (port, rx) = start()?;
    if let Some(rx) = rx {
        app.control_rx = Some(rx);
    }
    app.kopano.control_port = Some(port);
    Ok(port)
}

fn text_param(p: &Value, key: &str, fallback: &str) -> String {
    p.get(key).and_then(Value::as_str).unwrap_or(fallback).trim().to_string()
}

fn set_job(app: &LightcraftApp, ok: bool, text: impl Into<String>) {
    kopano::lock(&app.kopano.client.shared).job = Some(Job { busy: false, ok, text: text.into(), project: None });
}

/// The collaboration commands; `None` for any other id.
pub fn command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "kopano.panel" => {
            app.kopano.open = p.get("open").and_then(Value::as_bool).unwrap_or(!app.kopano.open);
            if let Some(a) = p.get("advanced").and_then(Value::as_bool) {
                app.kopano.advanced = a;
            }
            if app.kopano.open {
                let ctx = app_ctx(app);
                app.kopano.client.start(&ctx);
            }
            Ok(json!({"open": app.kopano.open, "advanced": app.kopano.advanced}))
        }
        "kopano.signIn" => {
            // ThabangVision sign-in (the device certificate) is not built yet
            let text = "Sign-in is coming soon.";
            set_job(app, true, text);
            Ok(json!({"message": text}))
        }
        "kopano.share" => share(app, p),
        "kopano.join" => join(app, p),
        "kopano.mode" => {
            let m = text_param(p, "mode", "approve");
            if m != "live" && m != "approve" {
                return Some(Err("The way to work is live or approve".to_string()));
            }
            app.kopano.share_mode = m;
            Ok(json!({"mode": app.kopano.share_mode}))
        }
        "kopano.approve" => review_op(app, "approve", p),
        "kopano.decline" => review_op(app, "decline", p),
        "kopano.view" => review_op(app, "view", p),
        "kopano.choose" => {
            let (project, member, key) = (text_param(p, "project", ""), text_param(p, "member", ""), text_param(p, "key", ""));
            let pick = text_param(p, "pick", "member");
            if project.is_empty() || member.is_empty() || key.is_empty() || (pick != "member" && pick != "main") {
                return Some(Err("Say which library, whose changes, which photo setting, and member or main".to_string()));
            }
            app.kopano.choices.insert((project, member, key), pick.clone());
            Ok(json!({"pick": pick}))
        }
        "kopano.setApprover" => {
            let req = json!({"op": "set-approver", "project": text_param(p, "project", ""), "account": text_param(p, "account", ""),
                "approver": p.get("approver").and_then(Value::as_bool).unwrap_or(true)});
            let ctx = app_ctx(app);
            app.kopano.client.request(&ctx, "Saving…", req).map(|()| json!({"started": true}))
        }
        "kopano.resolve" => {
            let req = json!({"op": "resolve", "project": text_param(p, "project", ""), "clash": text_param(p, "clash", ""), "pick": text_param(p, "pick", "a")});
            let ctx = app_ctx(app);
            app.kopano.client.request(&ctx, "Settling…", req).map(|()| json!({"started": true}))
        }
        "kopano.status" => {
            let state = app.kopano.client.state();
            let mut v = serde_json::to_value(&state).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("shareMode".into(), json!(app.kopano.share_mode));
                o.insert("open".into(), json!(app.kopano.open));
                o.insert("advanced".into(), json!(app.kopano.advanced));
                o.insert("controlPort".into(), json!(app.kopano.control_port));
                o.insert("apiPort".into(), json!(app.kopano.client.config.port));
            }
            Ok(v)
        }
        _ => return None,
    })
}

/// `kopano.share {name, account}`: share this library; `account` names the teammates who may
/// join (comma separated). Without params, the window's fields.
fn share(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let name = text_param(p, "name", &app.kopano.share_name);
    let accounts: Vec<String> = text_param(p, "account", &app.kopano.share_account)
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect();
    if name.is_empty() {
        return Err("Give the shared library a name".to_string());
    }
    if accounts.is_empty() {
        return Err("Type your teammate's account, so they may join".to_string());
    }
    let mode = text_param(p, "mode", &app.kopano.share_mode);
    if mode != "live" && mode != "approve" {
        return Err("The way to work is live or approve".to_string());
    }
    let port = control_port(app)?;
    let members: Vec<Value> = accounts.iter().map(|a| json!({"account": a, "role": "editor", "lanes": []})).collect();
    let req = json!({"op": "create-project", "name": name, "app": {"kind": "light-control", "port": port}, "members": members, "mode": mode});
    let ctx = app_ctx(app);
    app.kopano.client.request(&ctx, "Sharing…", req)?;
    Ok(json!({"started": true, "controlPort": port}))
}

/// `kopano.approve|decline|view {project, member, version, show}`: what the review buttons run.
/// Approve sends the owner's picks for the clashes of that member's changes.
fn review_op(app: &mut LightcraftApp, op: &str, p: &Value) -> Result<Value, String> {
    let project = text_param(p, "project", "");
    let member = text_param(p, "member", "");
    let show = text_param(p, "show", "member");
    if project.is_empty() || (member.is_empty() && show != "off") {
        return Err("Say which library and whose changes".to_string());
    }
    let mut req = json!({"op": op, "project": project, "member": member});
    if let Some(v) = p.get("version").and_then(Value::as_str) {
        req["version"] = json!(v);
    }
    let busy = match op {
        "approve" => {
            let mut choices = serde_json::Map::new();
            for ((pid, who, key), pick) in &app.kopano.choices {
                if *pid == project && *who == member {
                    choices.insert(key.clone(), json!(pick));
                }
            }
            if let Some(c) = p.get("choices").and_then(Value::as_object) {
                choices.extend(c.clone());
            }
            req["choices"] = Value::Object(choices);
            "Approving…"
        }
        "decline" => "Declining…",
        _ => {
            req["show"] = json!(show);
            "Showing…"
        }
    };
    let ctx = app_ctx(app);
    app.kopano.client.request(&ctx, busy, req)?;
    if op == "approve" || op == "decline" {
        // only this member's picks: another member's changes may still wait with their own
        app.kopano.choices.retain(|(pid, who, _), _| *pid != project || *who != member);
    }
    Ok(json!({"started": true}))
}

/// `kopano.join {project, from}`: join a teammate's shared library into this window.
fn join(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let project = text_param(p, "project", &app.kopano.join_project).to_lowercase();
    let from = text_param(p, "from", &app.kopano.join_from).to_lowercase();
    if !kopano::is_hex_id(&project) {
        return Err("The project id is 64 letters and digits (your teammate copies it under Advanced)".to_string());
    }
    if !kopano::is_hex_id(&from) {
        return Err("The teammate's device id is 64 letters and digits (they copy it under Advanced)".to_string());
    }
    let port = control_port(app)?;
    let req = json!({"op": "join", "project": project, "from": from, "app": {"kind": "light-control", "port": port}});
    let ctx = app_ctx(app);
    app.kopano.client.request(&ctx, "Joining…", req)?;
    Ok(json!({"started": true, "controlPort": port}))
}

/// Two people, drawn in code (our own original drawing): the toolbar's Share & Collaborate icon.
pub fn paint_icon(p: &egui::Painter, r: Rect, color: Color32) {
    let k = r.width().min(r.height()) / 18.0;
    let at = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
    let s = Stroke::new(1.4 * k.max(0.7), color);
    // a shoulder line: a half ellipse under a head
    let shoulders = |cx: f32, top: f32, w: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|i| {
                let a = std::f32::consts::PI * (i as f32 / 12.0);
                at(cx - w * a.cos(), top + 5.0 - 5.0 * a.sin())
            })
            .collect()
    };
    // the one behind, then the one in front
    p.circle_stroke(at(12.0, 5.2), 2.4 * k, s);
    p.add(egui::Shape::line(shoulders(12.5, 10.5, 4.2), s));
    p.circle_stroke(at(6.8, 6.6), 2.8 * k, s);
    p.add(egui::Shape::line(shoulders(6.8, 12.6, 5.0), s));
}

/// The toolbar button (top bar, right): opens or closes the window.
pub fn toolbar_button(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, egui::Id::new(("top", "collaborate")), Sense::click()).on_hover_text(crate::i18n::tr(TITLE));
    register(ui.ctx(), "icon:collaborate", r);
    let color = if resp.hovered() || app.kopano.open { t.text } else { t.icon };
    paint_icon(ui.painter(), r.shrink(5.0), color);
    if resp.clicked() {
        let _ = app.run("kopano.panel", json!({}));
    }
}

fn dot(ui: &mut egui::Ui, on: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, if on { GREEN } else { t.text_disabled });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(crate::i18n::tr(text)).font(t.semibold(12.0)).color(t.text));
}

fn dim(ui: &mut egui::Ui, text: impl Into<String>) {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(RichText::new(text.into()).color(t.text_dim).size(11.5)).wrap());
}

/// Copy `text` to the clipboard (and say so).
fn copy(app: &mut LightcraftApp, ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
    app.copied = Some(text.to_string());
    app.toast(ctx, crate::i18n::tr("Copied"));
}

/// Run a button's command; a refusal shows on the window's last line.
fn press(app: &mut LightcraftApp, id: &str) {
    press_with(app, id, json!({}));
}

fn press_with(app: &mut LightcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        set_job(app, false, e);
    }
}

/// A photo in plain words: its file name in this library, else the start of its key.
fn photo_name(app: &LightcraftApp, key: &str) -> String {
    let hash = key.split('|').next().unwrap_or(key);
    app.session
        .catalog
        .photos()
        .find(|p| p.content_hash.as_deref() == Some(hash) || format!("{}|{}", p.file_name, p.captured.as_deref().unwrap_or_default()) == key)
        .map(|p| p.file_name.clone())
        .unwrap_or_else(|| kopano::short(key).to_string())
}

/// The "I approve changes" part of one library: who approves, what waits, where my work is.
fn review_section(app: &mut LightcraftApp, ui: &mut egui::Ui, p: &kopano::Project, r: &kopano::Review) {
    let t = Tokens::get(ui.ctx());
    let tag = kopano::short(&p.id).to_string();
    let who = if r.is_owner {
        crate::i18n::tr("You approve changes").to_string()
    } else {
        format!("{} {}", r.owner_name, crate::i18n::tr("approves changes"))
    };
    dim(ui, who);
    if let Some(text) = r.mine_text() {
        let declined = r.mine.as_ref().is_some_and(|m| m.state == "declined");
        ui.add(egui::Label::new(RichText::new(text).color(if declined { t.caution } else { t.text_label }).size(11.5)).wrap());
    }
    if !r.approver {
        return;
    }
    heading(ui, "Changes to review");
    if r.pending.is_empty() {
        dim(ui, crate::i18n::tr("Nothing waiting."));
    }
    for e in &r.pending {
        let photos = if e.photos == 1 { "1 photo".to_string() } else { format!("{} photos", e.photos) };
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} changed {photos}", e.name)).color(t.text_label));
        });
        let viewing = r.viewing.as_ref().filter(|v| v.account == e.account);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let base = json!({"project": p.id, "member": e.account, "version": e.version});
            if viewing.is_none() {
                if text_button(ui, &format!("kopano-view-{tag}-{}", e.account), "View", false).clicked() {
                    let mut q = base.clone();
                    q["show"] = json!("member");
                    press_with(app, "kopano.view", q);
                }
            } else {
                let side = viewing.map(|v| v.side.as_str()).unwrap_or("member");
                if text_button(ui, &format!("kopano-before-{tag}"), "Before", side == "main").clicked() {
                    let mut q = base.clone();
                    q["show"] = json!("main");
                    press_with(app, "kopano.view", q);
                }
                if text_button(ui, &format!("kopano-after-{tag}"), "After", side == "member").clicked() {
                    let mut q = base.clone();
                    q["show"] = json!("member");
                    press_with(app, "kopano.view", q);
                }
                if text_button(ui, &format!("kopano-viewDone-{tag}"), "Done", false).clicked() {
                    press_with(app, "kopano.view", json!({"project": p.id, "show": "off"}));
                }
            }
            if text_button(ui, &format!("kopano-approve-{tag}-{}", e.account), "Approve", false).clicked() {
                press_with(app, "kopano.approve", base.clone());
            }
            if text_button(ui, &format!("kopano-decline-{tag}-{}", e.account), "Decline", false).clicked() {
                press_with(app, "kopano.decline", base.clone());
            }
        });
        if let Some(v) = viewing {
            let side = if v.side == "main" { crate::i18n::tr("your version (before)").to_string() } else { format!("{}'s version (after)", e.name) };
            dim(ui, format!("{} {side}. {}", crate::i18n::tr("This window shows"), crate::i18n::tr("Editing waits until you press Done.")));
        }
        for c in &e.clashes {
            let pick = app.kopano.choices.get(&(p.id.clone(), e.account.clone(), c.key.clone())).cloned().unwrap_or_else(|| "member".to_string());
            dim(
                ui,
                format!(
                    "{} · {}: {} {} · {} {}",
                    photo_name(app, &c.photo),
                    kopano::setting_text(&c.setting),
                    crate::i18n::tr("yours"),
                    c.main,
                    e.name,
                    c.member
                ),
            );
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let id = kopano::short(&c.photo).to_string();
                if text_button(ui, &format!("kopano-keepMain-{tag}-{id}-{}", e.account), "Keep yours", pick == "main").clicked() {
                    press_with(app, "kopano.choose", json!({"project": p.id, "member": e.account, "key": c.key, "pick": "main"}));
                }
                if text_button(ui, &format!("kopano-takeMember-{tag}-{id}-{}", e.account), "Take theirs", pick == "member").clicked() {
                    press_with(app, "kopano.choose", json!({"project": p.id, "member": e.account, "key": c.key, "pick": "member"}));
                }
            });
        }
    }
    if !r.declined.is_empty() {
        dim(ui, format!("{} {}", crate::i18n::tr("Declined:"), r.declined.join(", ")));
    }
}

/// A live library's clashes: both values of the setting, pick one.
fn clash_section(app: &mut LightcraftApp, ui: &mut egui::Ui, p: &kopano::Project) {
    let t = Tokens::get(ui.ctx());
    heading(ui, "Clashes");
    dim(ui, crate::i18n::tr("You and a teammate changed the same setting at once. Pick the one to keep."));
    for (i, c) in p.clashes.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} · {}", photo_name(app, &c.photo), kopano::setting_text(&c.setting))).color(t.text_label));
        });
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let tag = kopano::short(&p.id);
            if text_button(ui, &format!("kopano-pickA-{tag}-{i}"), &c.a, false).clicked() {
                press_with(app, "kopano.resolve", json!({"project": p.id, "clash": c.id, "pick": "a"}));
            }
            if text_button(ui, &format!("kopano-pickB-{tag}-{i}"), &c.b, false).clicked() {
                press_with(app, "kopano.resolve", json!({"project": p.id, "clash": c.id, "pick": "b"}));
            }
        });
    }
}

/// How often the background work runs, and how often this library's photos are listed again.
const TICK_MS: f64 = 2000.0;
const LIST_MS: f64 = 30000.0;

/// This library's photos (as the service keys them), listed at most every 30 s.
fn our_photos(app: &mut LightcraftApp, now: f64) -> BTreeSet<String> {
    if now - app.kopano.ours.0 >= LIST_MS {
        // the service's photo key: the content hash, else "file name|capture time" (virtual copies
        // are left out: they live with their original)
        let hashes = app
            .session
            .catalog
            .photos()
            .filter(|p| !p.deleted && !p.local && p.copy_of.is_none())
            .map(|p| p.content_hash.clone().unwrap_or_else(|| format!("{}|{}", p.file_name, p.captured.as_deref().unwrap_or_default())))
            .collect();
        app.kopano.ours = (now, hashes);
    }
    app.kopano.ours.1.clone()
}

/// Background work each frame, open or not (only where the window can be reached: the desktop
/// app): follow the service, remember which photos each library this window owns holds, and after
/// a restart on a new port link the libraries this library holds back to this window (`attach`).
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.kopano.start_control.is_none() && app.kopano.control_port.is_none() {
        return;
    }
    let now = crate::now_ms();
    if now - app.kopano.last_tick < TICK_MS {
        return;
    }
    app.kopano.last_tick = now;
    app.kopano.client.start(ctx);
    let state = app.kopano.client.state();
    let Some(home) = app.kopano.client.config.home.clone().filter(|_| state.running) else { return };
    let mine = app.kopano.control_port;
    for p in &state.projects {
        let Some(port) = p.app_port else { continue };
        if Some(port) == mine {
            let ours = our_photos(app, now);
            if app.kopano.recorded.get(&p.id) != Some(&ours) {
                match kopano::write_record(&home, &p.id, &ours) {
                    Ok(()) => {
                        app.kopano.recorded.insert(p.id.clone(), ours);
                    }
                    Err(e) => log::warn!("collaboration: could not remember the photos of {}: {e}", p.name),
                }
            }
        } else if p.app_alive == Some(false) && !app.kopano.attached.contains(&p.id) {
            let ours = our_photos(app, now);
            if !kopano::holds(&kopano::read_record(&home, &p.id), &ours) {
                continue;
            }
            app.kopano.attached.insert(p.id.clone());
            match control_port(app) {
                Ok(port) => app.kopano.client.attach(ctx, &p.id, &p.name, port),
                Err(error) => kopano::lock(&app.kopano.client.shared).note(Some(p.id.clone()), kopano::Said::AppProblem { error }, now),
            }
        }
    }
}

pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    tick(app, ctx);
    if !app.kopano.open {
        return;
    }
    app.kopano.client.start(ctx);
    let state = app.kopano.client.state();
    let mut open = true;
    let screen = ctx.content_rect();
    // left of the right-hand panels, so the Edit sliders stay in view
    let x = (screen.right() - Tokens::get(ctx).panel_w - 400.0).max(screen.left() + 8.0);
    let shown = egui::Window::new(crate::i18n::tr(TITLE))
        .id(egui::Id::new("kopano-panel"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(egui::pos2(x, screen.top() + 70.0))
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.set_width(340.0);
            ui.spacing_mut().item_spacing.y = 5.0;
            body(app, ui, &state);
        });
    if let Some(r) = shown {
        register(ctx, "region:collaborate", r.response.rect);
    }
    if !open {
        app.kopano.open = false;
    }
    // "3 s ago" keeps counting
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
}

fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, state: &State) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    match state.me.as_ref().filter(|_| state.running) {
        Some(me) if state.signed_in => {
            ui.horizontal(|ui| {
                dot(ui, true);
                ui.label(
                    RichText::new(format!("{} {}", crate::i18n::tr("Signed in to ThabangVision as"), me.name)).font(t.semibold(12.5)).color(t.text),
                );
            });
        }
        // the service runs, but this computer's ThabangVision sign-in is missing or ran out
        Some(_) => {
            dim(ui, crate::i18n::tr("Work on this library together with your team."));
            if text_button(ui, "kopano-signIn", "Sign in with ThabangVision", false).clicked() {
                press(app, "kopano.signIn");
            }
        }
        None => {
            ui.horizontal(|ui| {
                dot(ui, false);
                ui.label(RichText::new(crate::i18n::tr(kopano::NOT_RUNNING)).font(t.semibold(12.5)).color(t.text));
            });
            dim(ui, crate::i18n::tr("Start ThabangVision collaboration on this computer. This window finds it by itself."));
            if let Some(p) = &state.problem
                && p != kopano::NOT_RUNNING
            {
                dim(ui, kopano::plain(p));
            }
        }
    }

    heading(ui, "Shared libraries");
    if state.projects.is_empty() {
        dim(ui, crate::i18n::tr("Nothing shared yet."));
    }
    for p in &state.projects {
        ui.label(RichText::new(&p.name).font(t.semibold(12.0)).color(t.text_label));
        if p.teammates.is_empty() {
            dim(ui, crate::i18n::tr("No teammate has joined yet."));
        }
        for m in &p.teammates {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let on = m.online && state.running;
                dot(ui, on);
                ui.label(RichText::new(m.label()).color(t.text_label));
                ui.label(RichText::new(crate::i18n::tr(if on { "online" } else { "offline" })).color(t.text_dim).size(11.5));
            });
        }
        match &p.review {
            Some(r) => review_section(app, ui, p, r),
            None if p.mode.as_deref() == Some("live") => dim(ui, crate::i18n::tr("Works live: every edit goes to everyone.")),
            None => {}
        }
        if !p.clashes.is_empty() {
            clash_section(app, ui, p);
        }
    }

    heading(ui, "Recent activity");
    if state.activity.is_empty() {
        dim(ui, crate::i18n::tr("Nothing yet."));
    }
    let now = crate::now_ms();
    for a in state.activity.iter().take(SHOWN) {
        let times = if a.times > 1 { format!(" · {} times", a.times) } else { String::new() };
        let line = format!("{}{times} · {}", a.said.text(), kopano::ago((now - a.at_ms) / 1000.0));
        let color =
            if matches!(a.said, kopano::Said::AppProblem { .. } | kopano::Said::Merged { clashes: 1.., .. }) { t.caution } else { t.text_label };
        ui.add(egui::Label::new(RichText::new(line).color(color).size(11.5)).wrap());
    }

    ui.add_space(4.0);
    ui.separator();
    heading(ui, "Share this library");
    TextField::singleline("kopano:shareName", &mut app.kopano.share_name).hint(crate::i18n::tr("Name for the shared library")).width(330.0).show(ui);
    TextField::singleline("kopano:shareAccount", &mut app.kopano.share_account)
        .hint(crate::i18n::tr("Teammate's ThabangVision account"))
        .width(330.0)
        .show(ui);
    ui.horizontal(|ui| {
        let approve = app.kopano.share_mode != "live";
        if text_button(ui, "kopano-modeApprove", "I approve changes", approve).clicked() {
            app.kopano.share_mode = "approve".to_string();
        }
        if text_button(ui, "kopano-modeLive", "Work together live", !approve).clicked() {
            app.kopano.share_mode = "live".to_string();
        }
    });
    dim(
        ui,
        crate::i18n::tr(if app.kopano.share_mode == "live" {
            "Every edit goes to everyone at once."
        } else {
            "Your teammates see their own edits at once. Your library changes only when you approve."
        }),
    );
    if text_button(ui, "kopano-share", "Share", false).clicked() {
        press(app, "kopano.share");
    }

    ui.add_space(4.0);
    let header = egui::CollapsingHeader::new(RichText::new(crate::i18n::tr("Advanced (testing)")).color(t.text_dim).size(11.5))
        .id_salt("kopano-advanced")
        .open(Some(app.kopano.advanced))
        .show(ui, |ui| advanced(app, ui, state, &ctx));
    register(&ctx, "button:kopano-advanced", header.header_response.rect);
    if header.header_response.clicked() {
        app.kopano.advanced = !app.kopano.advanced;
    }

    if let Some(job) = &state.job {
        ui.add_space(4.0);
        let color = if job.busy {
            t.text_dim
        } else if job.ok {
            t.text_label
        } else {
            t.caution
        };
        let text = if job.ok || job.busy { job.text.clone() } else { kopano::plain(&job.text) };
        ui.add(egui::Label::new(RichText::new(text).color(color).size(11.5)).wrap());
    }
}

/// Ids and the id-based Join, until ThabangVision invites by username replace them.
fn advanced(app: &mut LightcraftApp, ui: &mut egui::Ui, state: &State, ctx: &egui::Context) {
    let t = Tokens::get(ui.ctx());
    dim(ui, crate::i18n::tr("For testing: ThabangVision invites by username will replace these ids."));
    if let Some(me) = &state.me {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", crate::i18n::tr("Your device id"), kopano::short(&me.device))).color(t.text_dim).monospace());
            if text_button(ui, "kopano-copyDevice", "Copy", false).clicked() {
                copy(app, ctx, &me.device);
            }
        });
    }
    for p in &state.projects {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", p.name, kopano::short(&p.id))).color(t.text_dim).monospace());
            if text_button(ui, &format!("kopano-copyProject-{}", kopano::short(&p.id)), "Copy", false).clicked() {
                copy(app, ctx, &p.id);
            }
        });
    }
    heading(ui, "Join a shared library");
    TextField::singleline("kopano:joinProject", &mut app.kopano.join_project).hint(crate::i18n::tr("Project id")).width(320.0).show(ui);
    TextField::singleline("kopano:joinFrom", &mut app.kopano.join_from).hint(crate::i18n::tr("Teammate's device id")).width(320.0).show(ui);
    if text_button(ui, "kopano-join", "Join", false).clicked() {
        press(app, "kopano.join");
    }
}
