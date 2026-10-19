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
//! The service reaches this window through Light's control port. With `--control PORT` that is
//! the port; without it, the first Share or Join starts the control server on demand
//! ([`StartControl`], supplied by the desktop app) and hands its port to the service.

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
    pub join_project: String,
    pub join_from: String,
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
            join_project: String::new(),
            join_from: String::new(),
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
        return Err("This window can't be shared: it has no control port (start Light with --control PORT)".to_string());
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
        "kopano.status" => {
            let state = app.kopano.client.state();
            let mut v = serde_json::to_value(&state).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
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
    let port = control_port(app)?;
    let members: Vec<Value> = accounts.iter().map(|a| json!({"account": a, "role": "editor", "lanes": []})).collect();
    let req = json!({"op": "create-project", "name": name, "app": {"kind": "light-control", "port": port}, "members": members});
    let ctx = app_ctx(app);
    app.kopano.client.request(&ctx, "Sharing…", req)?;
    Ok(json!({"started": true, "controlPort": port}))
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
    if let Err(e) = app.run(id, json!({})) {
        set_job(app, false, e);
    }
}

pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
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
    let me = state.me.as_ref().filter(|_| state.running);
    match me {
        Some(me) => {
            ui.horizontal(|ui| {
                dot(ui, true);
                ui.label(
                    RichText::new(format!("{} {}", crate::i18n::tr("Signed in to ThabangVision as"), me.name)).font(t.semibold(12.5)).color(t.text),
                );
            });
        }
        None if !state.signed_in => {
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
                dim(ui, p.clone());
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
        ui.add(egui::Label::new(RichText::new(&job.text).color(color).size(11.5)).wrap());
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
