//! RenderPRO — conversor de codecs para o PLAYERSLICE e o VIDEOHOST PLAYER.
//!
//! Interface em HTML/JS (pasta ui/); aqui ficam os comandos que a interface chama.
//! O trabalho pesado é feito pelo ffmpeg/ffprobe incluídos no instalador.

mod media;
mod presets;
mod render;
mod sys;
#[cfg(test)]
mod itest;

use base64::Engine;
use futures_util::StreamExt;
use media::MediaInfo;
use presets::{Plan, Player, RenderSettings};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

const REPO: &str = "NunesDanielPT/RenderPRO";
const SETUP_ASSET: &str = "RenderPRO-Setup.exe";

// ---------------------------------------------------------------- informação

#[derive(Serialize)]
struct AppInfo {
    version: String,
    windows: bool,
    ffmpeg: Option<String>,
    bundled: bool,
}

#[tauri::command]
async fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        windows: cfg!(windows),
        ffmpeg: render::ffmpeg_version().await,
        bundled: render::tools().bundled,
    }
}

#[derive(Serialize)]
struct CodecInfo {
    id: &'static str,
    label: &'static str,
    alpha: bool,
}

#[tauri::command]
fn codecs(player: Player) -> Vec<CodecInfo> {
    presets::player_codecs(player)
        .iter()
        .map(|c| CodecInfo { id: c.id(), label: c.label(), alpha: c.alpha() })
        .collect()
}

// ---------------------------------------------------------------- ficheiros

fn is_video(p: &Path) -> bool {
    p.extension()
        .map(|e| media::VIDEO_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<String>) {
    if depth > 4 || out.len() > 5000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') || name.ends_with(".renderpro-part") {
            continue;
        }
        if p.is_dir() {
            walk(&p, depth + 1, out);
        } else if is_video(&p) {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}

/// Ficheiros arrastados/escolhidos → lista de vídeos (pastas são percorridas).
#[tauri::command]
fn expand_paths(paths: Vec<String>) -> Vec<String> {
    let mut out = vec![];
    for p in paths {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            walk(&pb, 0, &mut out);
        } else if pb.is_file() && !p.ends_with(".renderpro-part") {
            out.push(p);
        }
    }
    out
}

#[tauri::command]
async fn probe_file(path: String) -> Result<MediaInfo, String> {
    render::probe(&path).await
}

static THUMBS: Semaphore = Semaphore::const_new(2);

#[tauri::command]
async fn thumbnail(path: String, at: f64) -> Result<String, String> {
    let _permit = THUMBS.acquire().await.map_err(|e| e.to_string())?;
    let png = render::thumbnail(&path, at).await?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

#[tauri::command]
fn plan_files(infos: Vec<MediaInfo>, settings: RenderSettings) -> Vec<Plan> {
    infos.iter().map(|i| render::plan_for(i, &settings)).collect()
}

#[tauri::command]
fn free_space(path: String) -> Option<u64> {
    sys::free_space(Path::new(&path))
}

// ---------------------------------------------------------------- render

#[tauri::command]
async fn render_file(
    app: AppHandle,
    id: String,
    path: String,
    settings: RenderSettings,
) -> Result<render::JobResult, String> {
    render::render_job(app, id, path, settings).await
}

#[tauri::command]
fn cancel_job(id: String) -> bool {
    render::cancel(&id)
}

#[tauri::command]
fn cancel_all() {
    render::cancel_all();
}

#[tauri::command]
fn force_quit(app: AppHandle) {
    render::cancel_all();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        // dá tempo para os ffmpeg morrerem e os temporários serem apagados
        for _ in 0..30 {
            if sys::active() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        app2.exit(0);
    });
}

// ---------------------------------------------------------------- abrir coisas

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("badurl".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_logs(app: AppHandle) -> Result<(), String> {
    let d = render::log_dir().ok_or("nolog")?;
    app.opener().open_path(d.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- atualizações

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("RenderPRO/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("net:{e}"))
}

#[derive(Serialize)]
struct ReleaseInfo {
    version: String,
    notes: String,
    url: Option<String>,
    size: u64,
}

#[tauri::command]
async fn latest_release() -> Result<ReleaseInfo, String> {
    let r = http()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("net:{e}"))?;
    if !r.status().is_success() {
        return Err(format!("http:{}", r.status().as_u16()));
    }
    let v: Value = r.json().await.map_err(|e| format!("json:{e}"))?;
    let asset = v["assets"].as_array().into_iter().flatten().find(|a| a["name"] == SETUP_ASSET);
    Ok(ReleaseInfo {
        version: v["tag_name"].as_str().unwrap_or_default().trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        url: asset.and_then(|a| a["browser_download_url"].as_str()).map(String::from),
        size: asset.and_then(|a| a["size"].as_u64()).unwrap_or(0),
    })
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Descarrega o instalador novo, fecha o RenderPRO, instala em silêncio e volta a abrir.
#[tauri::command]
async fn self_update(app: AppHandle, url: String, size: u64) -> Result<(), String> {
    if sys::active() > 0 {
        return Err("busy".into());
    }
    if !(url.starts_with("https://github.com/NunesDanielPT/") || url.contains(".githubusercontent.com/")) {
        return Err("badurl".into());
    }
    let dir = std::env::temp_dir().join("RenderPRO");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(SETUP_ASSET);
    let resp = http()?.get(&url).send().await.map_err(|e| format!("net:{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("http:{}", resp.status().as_u16()));
    }
    let total = resp.content_length().unwrap_or(size);
    let mut file = tokio::fs::File::create(&path).await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut got = 0u64;
    let mut last = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("net:{e}"))?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(150) {
            let _ = app.emit("renderpro://update", (got, total));
            last = std::time::Instant::now();
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if total > 0 && got != total {
        let _ = std::fs::remove_file(&path);
        return Err("incomplete".into());
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = format!(
        "Start-Sleep -Seconds 2; $p = Start-Process -FilePath {} -ArgumentList '/S' -Wait -PassThru; Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue; Start-Process -FilePath {}",
        ps_quote(&path.to_string_lossy()),
        ps_quote(&path.to_string_lossy()),
        ps_quote(&me.to_string_lossy())
    );
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(sys::creation_flags(false));
    }
    c.spawn().map_err(|e| format!("ps:{e}"))?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------- arranque

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            // "Abrir com RenderPRO" com a app já aberta: acrescenta os ficheiros à fila
            let files: Vec<String> = args.into_iter().skip(1).filter(|a| !a.starts_with('-')).collect();
            if !files.is_empty() {
                let _ = app.emit("renderpro://open-files", files);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let h = app.handle();
            render::init_log(h);
            render::init_tools(h);
            render::init_pending(h);
            sys::start_keep_awake_thread();
            let t = render::tools();
            render::log(&format!(
                "RenderPRO {} a arrancar; ffmpeg: {} (incluído: {})",
                h.package_info().version,
                t.ffmpeg.display(),
                t.bundled
            ));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if sys::active() > 0 {
                    api.prevent_close();
                    let _ = window.emit("renderpro://close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            codecs,
            expand_paths,
            probe_file,
            thumbnail,
            plan_files,
            free_space,
            render_file,
            cancel_job,
            cancel_all,
            force_quit,
            reveal,
            open_path,
            open_url,
            open_logs,
            latest_release,
            self_update,
            startup_files
        ])
        .run(tauri::generate_context!())
        .expect("erro ao arrancar o RenderPRO");
}

/// Ficheiros passados na linha de comandos ao arrancar ("Abrir com").
#[tauri::command]
fn startup_files() -> Vec<String> {
    std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect()
}
