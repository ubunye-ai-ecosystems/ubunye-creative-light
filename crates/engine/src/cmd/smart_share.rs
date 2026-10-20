//! Ubunye: sharing photos as smart previews, so a collaborator who does not have the original
//! sees (and edits) an accurate picture of it.
//!
//! - `photo.exportSmartPreview {id, path}` writes the photo's smart preview (the scene-linear
//!   proxy of [`crate::smart`], not a rendered JPEG) with the facts the receiver needs to render
//!   it as the original renders: content hash, file name, kind, format, size, as-shot white
//!   balance, embedded lens data and the camera fields (a `photo` object in the proxy's header).
//! - `library.addFromSmartPreview {smartPreview, contentHash?, fileName?, ...}` adds a photo whose
//!   original is offline: keyed by the ORIGINAL's content hash, rendered from the proxy. Edits
//!   apply as usual. Its file is a stand-in path ([`STAND_IN`]) that never exists on disk.
//! - When the original is later imported (any mode) or the photo is relinked to it, the original
//!   takes the photo over under the same id: settings, history, ratings and albums are kept.
//! - A session without a library on disk (`--memory`) keeps smart previews in a throwaway folder
//!   ([`Scratch`]), removed when the session ends.
//
// New file by Ubunye AI Ecosystems, 2026.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_catalog::{Catalog, MediaKind, Meta, Op, Photo, PhotoId, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

/// The start of a stand-in's file path: `smart-preview:/<content hash>/<file name>`. It never
/// names a file (on Windows the colon makes it an invalid path), so the photo counts as offline
/// and renders from its smart preview.
pub const STAND_IN: &str = "smart-preview:/";

/// The proxy header's photo facts are kept under this size, so the header stays within what
/// [`crate::smart::is_valid`] reads.
const FACTS_MAX: usize = 3500;

/// Whether `p` is shown from a shared smart preview (its original never was in this library).
pub fn is_stand_in(p: &Photo) -> bool {
    matches!(&p.source, Source::File { path } if path.starts_with(STAND_IN))
}

/// The stand-in file path for the original with content hash `hash` named `file_name`.
pub fn stand_in_path(hash: &str, file_name: &str) -> String {
    format!("{STAND_IN}{hash}/{file_name}")
}

/// The stand-ins in the library (not in Recently Deleted), by their original's content hash.
pub fn stand_ins_by_hash(cat: &Catalog) -> std::collections::HashMap<String, PhotoId> {
    cat.photos().filter(|p| is_stand_in(p) && !p.deleted).filter_map(|p| p.content_hash.clone().map(|h| (h, p.id))).collect()
}

/// `old` described by the file at `path` (named `name`) that `info` probed: its kind, format,
/// size, content, white balance and lens data follow the file; settings, history, ratings, flags,
/// labels and metadata are kept (camera fields the photo lacks are filled from the file).
pub fn adopt_file(old: &Photo, path: &str, name: &str, info: &crate::media::ProbeInfo) -> Photo {
    let mut q = old.clone();
    q.source = Source::File { path: path.to_string() };
    q.file_name = name.to_string();
    q.kind = info.kind;
    if !info.format.is_empty() {
        q.format = info.format.clone();
    }
    if info.width > 0 && info.height > 0 {
        (q.width, q.height) = (info.width, info.height);
    }
    q.file_size = info.file_size;
    q.content_hash = info.content_hash.clone().or(q.content_hash);
    q.as_shot_wb = info.as_shot_wb;
    q.embedded_lens = info.embedded_lens;
    q.preview_only = info.preview_only.clone();
    if q.captured.is_none() {
        q.captured = info.captured.clone();
    }
    let (m, src) = (&mut q.meta, &info.meta);
    for (dst, from) in [(&mut m.camera, &src.camera), (&mut m.lens, &src.lens), (&mut m.shutter, &src.shutter)] {
        if dst.is_empty() {
            dst.clone_from(from);
        }
    }
    m.focal_mm = m.focal_mm.or(src.focal_mm);
    m.aperture = m.aperture.or(src.aperture);
    m.iso = m.iso.or(src.iso);
    m.gps = m.gps.or(src.gps);
    q
}

/// Replace photo `q.id`'s record with `q` in one undoable step (existing ops only, so the catalog
/// format does not change; albums and stacks name photos by id and are untouched).
pub fn replace_op(q: Photo) -> Op {
    Op::Batch { ops: vec![Op::RemovePhoto { id: q.id }, Op::AddPhoto { photo: Box::new(q) }] }
}

/// The content hash without the `:dupN` suffix duplicates get.
fn plain_hash(p: &Photo) -> Option<&str> {
    p.content_hash.as_deref().map(|h| h.split(':').next().unwrap_or(h)).filter(|h| !h.is_empty())
}

/// What a receiver needs to render the proxy as the original renders (the `photo` header object).
fn facts(p: &Photo) -> Value {
    let m = &p.meta;
    let mut f = json!({
        "contentHash": plain_hash(p),
        "fileName": p.file_name,
        "format": p.format,
        "kind": p.kind,
        "width": p.width,
        "height": p.height,
        "fileSize": p.file_size,
        "captured": p.captured,
        "asShotWb": p.as_shot_wb,
        "previewOnly": p.preview_only,
        "meta": {"camera": m.camera, "lens": m.lens, "focal_mm": m.focal_mm, "aperture": m.aperture, "shutter": m.shutter, "iso": m.iso},
    });
    if let Some(l) = &p.embedded_lens
        && let Ok(v) = serde_json::to_value(l)
    {
        f["embeddedLens"] = v;
        if f.to_string().len() > FACTS_MAX {
            f.as_object_mut().map(|o| o.remove("embeddedLens"));
        }
    }
    f
}

/// Split a proxy into (its header object, the JPEG after it).
fn split(bytes: &[u8]) -> std::result::Result<(Value, &[u8]), String> {
    let rest = bytes.strip_prefix(crate::smart::MAGIC).ok_or("not a smart preview")?;
    let nl = rest.iter().position(|b| *b == b'\n').ok_or("bad smart preview")?;
    let head: Value = serde_json::from_slice(&rest[..nl]).map_err(|e| format!("bad smart preview header: {e}"))?;
    if !head.is_object() {
        return Err("bad smart preview header".into());
    }
    Ok((head, &rest[nl + 1..]))
}

/// `bytes` with `photo` facts in the header.
fn with_facts(bytes: &[u8], photo: Value) -> std::result::Result<Vec<u8>, String> {
    let (mut head, jpeg) = split(bytes)?;
    head["photo"] = photo;
    let mut out = crate::smart::MAGIC.to_vec();
    out.extend_from_slice(head.to_string().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(jpeg);
    Ok(out)
}

/// A throwaway smart previews folder (a session with no library on disk), removed on drop.
#[derive(Debug)]
pub struct Scratch {
    pub dir: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Session {
    /// Smart previews in a session with no library on disk (`--memory`): kept in a throwaway
    /// folder under the system temp folder, removed when the session ends.
    pub fn with_memory_smart_previews(mut self) -> Self {
        self.smart_dir_or_scratch();
        self
    }

    /// The smart previews folder: the library's, else (no library on disk) a throwaway one.
    pub(crate) fn smart_dir_or_scratch(&mut self) -> Option<PathBuf> {
        if let Some(d) = &self.media.smart_dir {
            return Some(d.clone());
        }
        if cfg!(target_arch = "wasm32") || self.library.as_ref().is_some_and(|l| l.on_disk) {
            return None;
        }
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ubunye-light-smart-previews-{}-{n}", std::process::id()));
        self.media.smart_dir = Some(dir.clone());
        self.smart_scratch = Some(Arc::new(Scratch { dir: dir.clone() }));
        Some(dir)
    }
}

/// Make the smart previews folder `dir` there (a chosen folder must be there already, never
/// recreated on another drive; the default and the throwaway one are created).
fn ready_dir(s: &Session, dir: &Path) -> std::result::Result<(), String> {
    if s.smart_previews_dir.is_some() {
        crate::smart::check_writable(dir)
    } else {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
    }
}

/// The photo's smart preview bytes: the library's current proxy, else one built from the
/// original (and kept in the library), else an out-of-date proxy when the original is offline.
fn proxy_bytes(s: &mut Session, ph: &Photo) -> std::result::Result<Vec<u8>, String> {
    let stored = s.smart_dir_or_scratch().map(|d| d.join(crate::smart::file_name(ph)));
    let valid = stored.as_ref().filter(|p| crate::smart::is_valid(p));
    if let Some(p) = valid.filter(|p| !crate::smart::is_stale(p)) {
        return std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    }
    let original = matches!(&ph.source, Source::File { path } if !is_stand_in(ph) && Path::new(path).is_file());

    if original {
        let src = s.media.origin_ref(&ph.source, crate::media::SourceLevel::Preview.max_edge()).load_source()?;
        let bytes = crate::smart::encode(&src.image, src.info_or(Default::default()).camera_tone.as_ref())?;
        // kept in the library too, as Build Smart Previews would (a failure here is not the export's)
        if let Some(p) = &stored
            && let Some(dir) = p.parent()
            && let Err(e) = ready_dir(s, dir).and_then(|()| lightcraft_catalog::safe_file::write_atomic(p, &bytes).map_err(|e| e.to_string()))
        {
            log::warn!("smart preview {}: {e}", p.display());
        }
        return Ok(bytes);
    }
    match valid {
        Some(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
        None => Err("the original is offline and the photo has no smart preview".into()),
    }
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "photo.exportSmartPreview";
    let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or(s.active()).ok_or_else(|| bad(C, "no photo (give `id`)"))?;
    let path = str_param(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(C, "missing `path`"))?;
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() {
        return Err(bad(C, "path must be an absolute file path"));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(bad(C, format!("{}: its folder does not exist", path.display())));
    }
    let ph = s.catalog.photo(id).map(|p| (**p).clone()).ok_or_else(|| bad(C, "no such photo"))?;
    if !matches!(ph.source, Source::File { .. }) || ph.kind == MediaKind::Video {
        return Err(bad(C, "only photos from files have smart previews"));
    }
    if plain_hash(&ph).is_none() {
        return Err(bad(C, "the photo has no content hash (reload it first)"));
    }
    let bytes = proxy_bytes(s, &ph).and_then(|b| with_facts(&b, facts(&ph))).map_err(|e| bad(C, e))?;
    lightcraft_catalog::safe_file::write_atomic(&path, &bytes).map_err(|e| bad(C, format!("{}: {e}", path.display())))?;
    let (head, _) = split(&bytes).map_err(|e| bad(C, e))?;
    Ok(json!({
        "id": id.0,
        "path": path.to_string_lossy(),
        "bytes": bytes.len(),
        "contentHash": plain_hash(&ph),
        "previewWidth": head["w"],
        "previewHeight": head["h"],
    }))
}

/// A content hash safe to use in a file path.
fn valid_hash(h: &str) -> bool {
    !h.is_empty() && h.len() <= 128 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.addFromSmartPreview";
    let sp = str_param(p, "smartPreview").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(C, "missing `smartPreview` (path to a .lcsp file)"))?;
    let sp = PathBuf::from(sp.trim());
    if !crate::smart::is_valid(&sp) {
        return Err(bad(C, format!("{}: not a complete smart preview", sp.display())));
    }
    let bytes = std::fs::read(&sp).map_err(|e| bad(C, format!("{}: {e}", sp.display())))?;
    let (head, _) = split(&bytes).map_err(|e| bad(C, e))?;
    // the pixels must decode before anything is added
    let (proxy, _) = crate::smart::decode(&bytes).map_err(|e| bad(C, e))?;
    let f = head.get("photo").cloned().unwrap_or_else(|| json!({}));
    let text = |k: &str| str_param(p, k).or_else(|| str_param(&f, k)).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let num = |k: &str| p.get(k).and_then(Value::as_u64).or_else(|| f.get(k).and_then(Value::as_u64));
    let hash = text("contentHash").ok_or_else(|| bad(C, "missing `contentHash` (the original's content hash)"))?;
    if !valid_hash(&hash) {
        return Err(bad(C, "contentHash must be letters, digits, - or _"));
    }
    if let Some(h) = str_param(&f, "contentHash")
        && h != hash
    {
        return Err(bad(C, format!("this smart preview is of the photo {h}, not {hash}")));
    }
    // already here (the original, or this preview added before): nothing to add
    if let Some(ex) = s.catalog.photos().find(|q| !q.deleted && plain_hash(q) == Some(hash.as_str())) {
        return Ok(json!({"id": ex.id.0, "added": false, "standIn": is_stand_in(ex)}));
    }
    let file_name = text("fileName").unwrap_or_else(|| format!("{hash}.jpg"));
    if file_name.contains(['/', '\\']) {
        return Err(bad(C, "fileName must be a file name, not a path"));
    }
    // the format follows a file name given here (else the stored facts, else the stored name)
    let ext = Path::new(&file_name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let format = match str_param(p, "format").or(str_param(p, "fileName").map(|_| ext.as_str())) {
        Some(f) => f.trim().to_string(),
        None => text("format").unwrap_or(ext),
    };
    let kind = match text("kind") {
        Some(k) => serde_json::from_value::<MediaKind>(json!(k.to_lowercase())).map_err(|_| bad(C, "kind must be raw or image"))?,
        None if lightcraft_catalog::model::relative_wb_format(&format) || format.eq_ignore_ascii_case("DNG") => MediaKind::Raw,
        None => MediaKind::Image,
    };
    if kind == MediaKind::Video {
        return Err(bad(C, "videos have no smart previews"));
    }
    let dim = |k: &str, d: usize| num(k).and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0).unwrap_or(d as u32);
    let (width, height) = (dim("width", proxy.width), dim("height", proxy.height));
    let dir = s.smart_dir_or_scratch().ok_or_else(|| bad(C, "smart previews are not available here"))?;
    let id = s.catalog.alloc_photo_id();
    let now = (s.clock)();
    let mut ph = Photo::new(id, Source::File { path: stand_in_path(&hash, &file_name) }, &file_name, &format, width, height, &now);
    ph.kind = kind;
    ph.file_size = num("fileSize").unwrap_or(0);
    ph.captured = text("captured");
    ph.content_hash = Some(hash.clone());
    ph.meta = f.get("meta").and_then(|m| serde_json::from_value::<Meta>(m.clone()).ok()).unwrap_or_default();
    ph.as_shot_wb = f.get("asShotWb").and_then(|v| serde_json::from_value(v.clone()).ok());
    ph.embedded_lens = f.get("embeddedLens").and_then(|v| serde_json::from_value(v.clone()).ok());
    ph.preview_only = str_param(&f, "previewOnly").map(str::to_string);
    crate::import::apply_import_defaults(s, &mut ph);
    // the proxy goes where this library keeps its smart previews (named by the original's content)
    let stored = dir.join(crate::smart::file_name(&ph));
    ready_dir(s, &dir)
        .and_then(|()| lightcraft_catalog::safe_file::write_atomic(&stored, &bytes).map_err(|e| format!("{}: {e}", stored.display())))
        .map_err(|e| bad(C, e))?;
    s.commit("Add Photo", Op::AddPhoto { photo: Box::new(ph) })?;
    Ok(json!({"id": id.0, "added": true, "standIn": true, "contentHash": hash, "smartPreview": stored.to_string_lossy()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "photo.exportSmartPreview",
            "Export Smart Preview",
            [],
            None,
            "{id?, path: absolute .lcsp file}: write the photo's smart preview (its scene-linear proxy, about 1-3 MB, from the library's smart preview or built from the original) with the facts needed to render it as the original renders (content hash, file name, kind, format, size, white balance, lens) → {id, path, bytes, contentHash, previewWidth, previewHeight}. Another library adds it with library.addFromSmartPreview",
            always,
            export
        ),
        cmd!(
            "library.addFromSmartPreview",
            "Add Photo from Smart Preview",
            [],
            None,
            "{smartPreview: path to a .lcsp file, contentHash?, fileName?, captured?, fileSize?, width?, height?, kind?: raw|image}: add a photo whose original is not here, keyed by the ORIGINAL's content hash and shown (and edited, and exported at preview size) from the smart preview; params override the facts photo.exportSmartPreview stored in the file. Its file is `smart-preview:/<hash>/<fileName>` (offline). When the original is imported later, or the photo is relinked to it, the original takes the photo over with all its settings → {id, added: true, standIn, contentHash, smartPreview: where it is kept}; a photo with that content hash already here → {id, added: false}",
            always,
            add
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-smart-share-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A small procedural PNG (distinct per seed).
    fn write_png(path: &Path, seed: u8) {
        let (w, h) = (96usize, 64usize);
        let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 2) as u8, (i / w * 3) as u8, seed, 255]).collect();
        let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn mean_abs(a: &lightcraft_raster::Rgba8, b: &lightcraft_raster::Rgba8) -> f64 {
        assert_eq!((a.width, a.height), (b.width, b.height));
        let sum: f64 = a.data.iter().zip(&b.data).map(|(x, y)| (0..3).map(|k| (x[k] as f64 - y[k] as f64).abs()).sum::<f64>()).sum();
        sum / (a.data.len() * 3) as f64
    }

    fn sel(s: &mut Session, id: PhotoId) {
        s.execute("library.select", &json!({"ids": [id.0], "active": id.0})).unwrap();
    }

    /// Owner side: a memory session with one imported file; returns (session, id, its .lcsp).
    fn owner(dir: &Path) -> (Session, PhotoId, PathBuf) {
        write_png(&dir.join("IMG_1.png"), 9);
        let mut a = Session::new().with_fs();
        a.execute("library.import", &json!({"paths": [dir.join("IMG_1.png").to_string_lossy()]})).unwrap();
        let id = a.active().unwrap();
        let out = dir.join("IMG_1.lcsp");
        let r = a.execute("photo.exportSmartPreview", &json!({"id": id.0, "path": out.to_string_lossy()})).unwrap();
        assert_eq!(r["contentHash"], json!(a.catalog.photo(id).unwrap().content_hash), "{r}");
        (a, id, out)
    }

    #[test]
    fn a_shared_smart_preview_renders_like_the_original_and_takes_edits() {
        let dir = temp("roundtrip");
        let (mut a, aid, lcsp) = owner(&dir);
        assert!(a.smart_scratch.is_some(), "a memory session gets a throwaway smart previews folder");
        let mut b = Session::new().with_fs();
        let r = b.execute("library.addFromSmartPreview", &json!({"smartPreview": lcsp.to_string_lossy()})).unwrap();
        assert_eq!(r["added"], true, "{r}");
        let bid = PhotoId(r["id"].as_u64().unwrap());
        let (pa, pb) = (a.catalog.photo(aid).unwrap().clone(), b.catalog.photo(bid).unwrap().clone());
        assert!(is_stand_in(&pb));
        assert_eq!(
            (pb.content_hash.as_ref(), pb.file_name.as_str(), pb.kind, pb.format.as_str()),
            (pa.content_hash.as_ref(), "IMG_1.png", pa.kind, pa.format.as_str())
        );
        assert_eq!((pb.width, pb.height, pb.file_size), (pa.width, pa.height, pa.file_size));
        // the same edit on both: the pictures match
        for (s, id) in [(&mut a, aid), (&mut b, bid)] {
            sel(s, id);
            s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
            s.execute("develop.set", &json!({"control": "light.highlights", "value": -60.0})).unwrap();
        }
        let ra = a.render_now(aid, 96, 64).unwrap().image;
        let rb = b.render_now(bid, 96, 64).expect("renders from the shared smart preview").image;
        let d = mean_abs(&ra, &rb);
        assert!(d < 2.0, "mean abs difference {d}/255");
        // adding it again does nothing; a wrong key is refused
        let again = b.execute("library.addFromSmartPreview", &json!({"smartPreview": lcsp.to_string_lossy()})).unwrap();
        assert_eq!((again["added"].as_bool(), again["id"].as_u64()), (Some(false), Some(bid.0)));
        assert!(b.execute("library.addFromSmartPreview", &json!({"smartPreview": lcsp.to_string_lossy(), "contentHash": "abc"})).is_err());
        // the receiver can hand it on (its proxy is in its own smart previews folder)
        let on = dir.join("again.lcsp");
        b.execute("photo.exportSmartPreview", &json!({"id": bid.0, "path": on.to_string_lossy()})).unwrap();
        let mut c = Session::new().with_fs();
        assert_eq!(c.execute("library.addFromSmartPreview", &json!({"smartPreview": on.to_string_lossy()})).unwrap()["added"], true);
        let scratch = b.smart_scratch.as_ref().unwrap().dir.clone();
        assert!(scratch.is_dir());
        drop(b);
        assert!(!scratch.exists(), "the throwaway folder goes with the session");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn params_override_and_missing_hash_is_refused() {
        let dir = temp("params");
        // a proxy without photo facts (as Build Smart Previews writes them)
        let img = lightcraft_scenes::demo_library()[0].render(64, 40);
        let bare = dir.join("bare.lcsp");
        std::fs::write(&bare, crate::smart::encode(&img, None).unwrap()).unwrap();
        let mut b = Session::new().with_fs();
        assert!(b.execute("library.addFromSmartPreview", &json!({"smartPreview": bare.to_string_lossy()})).is_err(), "no key");
        let r = b
            .execute(
                "library.addFromSmartPreview",
                &json!({"smartPreview": bare.to_string_lossy(), "contentHash": "00ff", "fileName": "DSC0001.ARW", "captured": "2026-10-01T10:00:00", "fileSize": 49_000_000u64, "width": 6000, "height": 4000, "kind": "raw"}),
            )
            .unwrap();
        let p = b.catalog.photo(PhotoId(r["id"].as_u64().unwrap())).unwrap();
        assert_eq!((p.kind, p.format.as_str(), p.width, p.height, p.file_size), (MediaKind::Raw, "ARW", 6000, 4000, 49_000_000));
        assert_eq!(p.captured.as_deref(), Some("2026-10-01T10:00:00"));
        assert!(b.render_now(p.id, 60, 40).is_ok());
        for bad_name in ["../x.ARW", "a/b.ARW"] {
            assert!(
                b.execute("library.addFromSmartPreview", &json!({"smartPreview": bare.to_string_lossy(), "contentHash": "01", "fileName": bad_name}))
                    .is_err()
            );
        }
        assert!(b.execute("library.addFromSmartPreview", &json!({"smartPreview": bare.to_string_lossy(), "contentHash": "../etc"})).is_err());
        let cut = dir.join("cut.lcsp");
        let all = std::fs::read(&bare).unwrap();
        std::fs::write(&cut, &all[..all.len() / 2]).unwrap();
        assert!(b.execute("library.addFromSmartPreview", &json!({"smartPreview": cut.to_string_lossy(), "contentHash": "02"})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_original_takes_over_on_import_with_settings_kept() {
        let dir = temp("takeover");
        let (_a, _aid, lcsp) = owner(&dir);
        let mut b = Session::new().with_fs();
        let r = b.execute("library.addFromSmartPreview", &json!({"smartPreview": lcsp.to_string_lossy()})).unwrap();
        let bid = PhotoId(r["id"].as_u64().unwrap());
        sel(&mut b, bid);
        b.execute("develop.set", &json!({"control": "light.exposure", "value": 0.75})).unwrap();
        b.execute("photo.rate", &json!({"ids": [bid.0], "rating": 4})).unwrap();
        let before = b.catalog.photo(bid).unwrap().develop.clone();
        // the original arrives (a copy of the owner's file, under another name)
        let got = dir.join("got");
        std::fs::create_dir_all(&got).unwrap();
        std::fs::copy(dir.join("IMG_1.png"), got.join("IMG_1.png")).unwrap();
        let r = b.execute("library.import", &json!({"paths": [got.to_string_lossy()]})).unwrap();
        assert_eq!(r["fromSmartPreview"], json!([bid.0]), "{r}");
        assert_eq!(b.catalog.len(), 1, "taken over, not added beside");
        let p = b.catalog.photo(bid).unwrap();
        assert!(!is_stand_in(p));
        assert_eq!(p.source, Source::File { path: got.join("IMG_1.png").to_string_lossy().to_string() });
        assert_eq!((p.rating, &p.develop), (4, &before), "settings and rating kept");
        assert!(b.render_now(bid, 96, 64).is_ok());
        // one undo puts the stand-in back
        b.execute("edit.undo", &json!({})).unwrap();
        assert!(is_stand_in(b.catalog.photo(bid).unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relink_follows_the_files_kind_and_format() {
        let dir = temp("relink");
        let (_a, _aid, lcsp) = owner(&dir);
        let mut b = Session::new().with_fs();
        let r = b
            .execute("library.addFromSmartPreview", &json!({"smartPreview": lcsp.to_string_lossy(), "fileName": "IMG_1.jpg", "kind": "image"}))
            .unwrap();
        let bid = PhotoId(r["id"].as_u64().unwrap());
        assert_eq!(b.catalog.photo(bid).unwrap().format, "JPG");
        sel(&mut b, bid);
        b.execute("develop.set", &json!({"control": "light.contrast", "value": 30.0})).unwrap();
        b.execute("photo.relink", &json!({"id": bid.0, "path": dir.join("IMG_1.png").to_string_lossy()})).unwrap();
        let p = b.catalog.photo(bid).unwrap();
        assert_eq!((p.format.as_str(), p.file_name.as_str(), is_stand_in(p)), ("PNG", "IMG_1.png", false));
        assert!(p.develop.light.contrast > 29.0, "settings kept");
        b.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(b.catalog.photo(bid).unwrap().format, "JPG");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
