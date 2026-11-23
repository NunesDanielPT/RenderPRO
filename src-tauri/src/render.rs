//! Execução do ffmpeg/ffprobe: análise, miniaturas, loudness, render e verificação.
//!
//! Regras de fiabilidade:
//! 1. O render escreve num ficheiro temporário ao lado do destino; só depois de verificado
//!    é renomeado para o nome final. Nunca aparece na pasta um vídeo meio escrito.
//! 2. Verificação: o ffprobe confirma codec, tamanho, duração e áudio; opcionalmente
//!    o ficheiro é descodificado do princípio ao fim (deteta frames corrompidos).
//! 3. Se o ffmpeg ficar parado (sem progresso) é terminado ao fim de STALL_SECS.
//! 4. Espaço em disco verificado antes de começar.
//! 5. O original nunca é alterado.

use crate::media::{parse_probe, MediaInfo};
use crate::presets::{build_plan, ffmpeg_args_with, nvenc_fits, Codec, Encoder, Loudness, Note, Plan, RenderSettings};
use crate::sys;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

const STALL_SECS: u64 = 120;

// ---------------------------------------------------------------- ferramentas

#[derive(Clone, Debug, Serialize)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub bundled: bool,
}

static TOOLS: OnceLock<Tools> = OnceLock::new();

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

pub fn init_tools(app: &AppHandle) {
    let mut dirs: Vec<PathBuf> = vec![];
    if let Ok(d) = std::env::var("RENDERPRO_FFMPEG_DIR") {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(r) = app.path().resource_dir() {
        dirs.push(r.join("ffmpeg"));
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(d) = me.parent() {
            dirs.push(d.join("ffmpeg"));
        }
    }
    let found = dirs.into_iter().find(|d| d.join(exe("ffmpeg")).is_file() && d.join(exe("ffprobe")).is_file());
    let tools = match found {
        Some(d) => Tools { ffmpeg: d.join(exe("ffmpeg")), ffprobe: d.join(exe("ffprobe")), bundled: true },
        None => Tools { ffmpeg: PathBuf::from(exe("ffmpeg")), ffprobe: PathBuf::from(exe("ffprobe")), bundled: false },
    };
    let _ = TOOLS.set(tools);
}

pub fn tools() -> Tools {
    TOOLS
        .get()
        .cloned()
        .unwrap_or(Tools { ffmpeg: PathBuf::from(exe("ffmpeg")), ffprobe: PathBuf::from(exe("ffprobe")), bundled: false })
}

fn command(program: &Path, low_priority: bool) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    c.creation_flags(sys::creation_flags(low_priority));
    #[cfg(not(windows))]
    let _ = low_priority;
    c
}

pub async fn ffmpeg_version() -> Option<String> {
    let out = command(&tools().ffmpeg, false).arg("-version").output().await.ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let first = txt.lines().next()?.to_string();
    // "ffmpeg version n7.1-... Copyright" → "n7.1-..."
    Some(first.split_whitespace().nth(2).unwrap_or(&first).to_string())
}

// ---------------------------------------------------------------- NVENC

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
pub struct GpuCaps {
    pub h264: bool,
    pub hevc: bool,
}

static GPU: OnceLock<GpuCaps> = OnceLock::new();

/// Testa o NVENC a sério (codifica 5 frames): o encoder estar compilado não chega —
/// é preciso uma gráfica NVIDIA e um driver recente.
pub async fn detect_gpu() -> GpuCaps {
    if let Some(c) = GPU.get() {
        return *c;
    }
    let caps = if std::env::var("RENDERPRO_FORCE_NVENC").is_ok() {
        GpuCaps { h264: true, hevc: true }
    } else {
        async fn works(enc: &str) -> bool {
            let fut = command(&tools().ffmpeg, false)
                .args([
                    "-hide_banner", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25", "-frames:v", "5",
                    "-pix_fmt", "yuv420p", "-c:v", enc, "-f", "null", "-",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            matches!(tokio::time::timeout(Duration::from_secs(20), fut).await, Ok(Ok(st)) if st.success())
        }
        GpuCaps { h264: works("h264_nvenc").await, hevc: works("hevc_nvenc").await }
    };
    log(&format!("NVENC: H.264 {} · H.265 {}", caps.h264, caps.hevc));
    let _ = GPU.set(caps);
    caps
}

pub fn gpu_caps() -> GpuCaps {
    GPU.get().copied().unwrap_or_default()
}

/// Testes: obriga a tentar o NVENC mesmo sem gráfica (para provar o recurso à CPU).
pub static FORCE_NVENC_TRY: AtomicBool = AtomicBool::new(false);

fn nvenc_for(c: Codec) -> bool {
    if FORCE_NVENC_TRY.load(Ordering::SeqCst) && matches!(c, Codec::H264 | Codec::H265) {
        return true;
    }
    let g = gpu_caps();
    match c {
        Codec::H264 => g.h264,
        Codec::H265 => g.hevc,
        _ => false,
    }
}

// ---------------------------------------------------------------- registo

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_log(app: &AppHandle) {
    if let Ok(d) = app.path().app_log_dir() {
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("renderpro.log");
        if std::fs::metadata(&p).map(|m| m.len() > 5_000_000).unwrap_or(false) {
            let _ = std::fs::rename(&p, d.join("renderpro.old.log"));
        }
        let _ = LOG_PATH.set(p);
    }
}

pub fn log_dir() -> Option<PathBuf> {
    LOG_PATH.get().and_then(|p| p.parent().map(Path::to_path_buf))
}

pub fn log(msg: &str) {
    use std::io::Write;
    let line = format!("{} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    if let Some(p) = LOG_PATH.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = f.write_all(line.as_bytes());
        }
    }
    #[cfg(debug_assertions)]
    eprint!("{line}");
}

// ---------------------------------------------------------------- temporários pendentes

static PENDING_FILE: OnceLock<PathBuf> = OnceLock::new();
static PENDING_LOCK: Mutex<()> = Mutex::new(());

pub fn init_pending(app: &AppHandle) {
    if let Ok(d) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("pending.txt");
        // Temporários de uma sessão anterior que acabou mal (crash, corte de energia).
        if let Ok(txt) = std::fs::read_to_string(&p) {
            for l in txt.lines().filter(|l| l.ends_with(".renderpro-part")) {
                if std::fs::remove_file(l).is_ok() {
                    log(&format!("Apagado temporário antigo: {l}"));
                }
            }
        }
        let _ = std::fs::write(&p, "");
        let _ = PENDING_FILE.set(p);
    }
}

fn pending_edit(path: &Path, add: bool) {
    let Some(f) = PENDING_FILE.get() else { return };
    let _g = PENDING_LOCK.lock();
    let mut lines: Vec<String> =
        std::fs::read_to_string(f).unwrap_or_default().lines().map(String::from).filter(|l| !l.is_empty()).collect();
    let s = path.to_string_lossy().to_string();
    lines.retain(|l| l != &s);
    if add {
        lines.push(s);
    }
    let _ = std::fs::write(f, lines.join("\n"));
}

// ---------------------------------------------------------------- cancelamento

#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}
impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
    pub fn is(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

pub static JOBS: Mutex<Option<HashMap<String, Arc<Cancel>>>> = Mutex::new(None);

fn register(id: &str) -> Arc<Cancel> {
    let c = Arc::new(Cancel::default());
    JOBS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id.to_string(), c.clone());
    c
}
fn unregister(id: &str) {
    if let Some(m) = JOBS.lock().unwrap().as_mut() {
        m.remove(id);
    }
}
pub fn cancel(id: &str) -> bool {
    if let Some(c) = JOBS.lock().unwrap().as_ref().and_then(|m| m.get(id)) {
        c.cancel();
        return true;
    }
    false
}
pub fn cancel_all() {
    if let Some(m) = JOBS.lock().unwrap().as_ref() {
        for c in m.values() {
            c.cancel();
        }
    }
}

// ---------------------------------------------------------------- correr o ffmpeg

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Progress {
    pub frame: u64,
    pub out_time: f64,
    pub fps: f64,
    pub speed: f64,
    pub end: bool,
}

/// Aplica uma linha de `-progress pipe:1`. Devolve true quando fecha um bloco.
pub fn parse_progress_line(line: &str, p: &mut Progress) -> bool {
    let Some((k, v)) = line.trim().split_once('=') else { return false };
    let v = v.trim();
    match k {
        "frame" => p.frame = v.parse().unwrap_or(p.frame),
        "fps" => p.fps = v.parse().unwrap_or(p.fps),
        // out_time_ms está (historicamente) em microssegundos, como out_time_us
        "out_time_us" | "out_time_ms" => {
            if let Ok(us) = v.parse::<i64>() {
                if us >= 0 {
                    p.out_time = us as f64 / 1_000_000.0;
                }
            }
        }
        "speed" => p.speed = v.trim_end_matches('x').parse().unwrap_or(p.speed),
        "progress" => {
            p.end = v == "end";
            return true;
        }
        _ => {}
    }
    false
}

#[derive(Debug)]
pub enum FfError {
    Cancelled,
    Stalled,
    Spawn(String),
    Failed(i32, Vec<String>),
}

impl FfError {
    fn code(&self) -> String {
        match self {
            FfError::Cancelled => "cancelled".into(),
            FfError::Stalled => "stalled".into(),
            FfError::Spawn(e) => format!("spawn:{e}"),
            FfError::Failed(c, lines) => {
                let tail: Vec<&String> = lines.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
                let msg = tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" | ");
                format!("ffmpeg:{c}:{msg}")
            }
        }
    }
}

/// Corre o ffmpeg com `-progress pipe:1` e devolve as linhas de erro (stderr).
async fn run_ff(
    args: &[String],
    low_priority: bool,
    cancel: &Cancel,
    mut on_progress: impl FnMut(&Progress),
) -> Result<Vec<String>, FfError> {
    let mut child = command(&tools().ffmpeg, low_priority)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| FfError::Spawn(e.to_string()))?;
    sys::adopt_child(child.id());

    let stderr = child.stderr.take().unwrap();
    let errs: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    let errs2 = errs.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let l = l.trim().to_string();
            if l.is_empty() {
                continue;
            }
            let mut q = errs2.lock().unwrap();
            if q.len() >= 200 {
                q.pop_front();
            }
            q.push_back(l);
        }
    });

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let mut p = Progress::default();
    let outcome: Result<(), FfError> = loop {
        if cancel.is() {
            break Err(FfError::Cancelled);
        }
        tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(STALL_SECS), lines.next_line()) => match r {
                Err(_) => break Err(FfError::Stalled),
                Ok(Ok(Some(line))) => {
                    if parse_progress_line(&line, &mut p) {
                        on_progress(&p);
                    }
                }
                Ok(_) => break Ok(()), // fim do stdout: o processo está a terminar
            },
            _ = cancel.notify.notified() => break Err(FfError::Cancelled),
        }
    };

    if let Err(e) = outcome {
        let _ = child.kill().await;
        let _ = child.wait().await;
        err_task.abort();
        return Err(e);
    }
    let status = match tokio::time::timeout(Duration::from_secs(STALL_SECS), child.wait()).await {
        Ok(Ok(s)) => s,
        _ => {
            let _ = child.kill().await;
            return Err(FfError::Stalled);
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), err_task).await;
    let lines: Vec<String> = errs.lock().unwrap().iter().cloned().collect();
    if !status.success() {
        return Err(FfError::Failed(status.code().unwrap_or(-1), lines));
    }
    Ok(lines)
}

// ---------------------------------------------------------------- análise

pub async fn probe(path: &str) -> Result<MediaInfo, String> {
    let meta = std::fs::metadata(path).map_err(|_| "missing".to_string())?;
    if !meta.is_file() {
        return Err("missing".into());
    }
    let fut = command(&tools().ffprobe, false)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path])
        .output();
    let out = tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .map_err(|_| "probe:timeout".to_string())?
        .map_err(|e| format!("spawn:{e}"))?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let e = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_string();
        format!("probe:{e}")
    })?;
    parse_probe(path, meta.len(), &v)
}

/// Miniatura PNG (com transparência, se existir).
pub async fn thumbnail(path: &str, at: f64) -> Result<Vec<u8>, String> {
    let t = format!("{:.3}", at.max(0.0));
    let mut child = command(&tools().ffmpeg, true)
        .args([
            "-hide_banner", "-v", "error", "-ss", &t, "-i", path, "-map", "0:v:0", "-frames:v", "1", "-vf",
            "scale=320:180:force_original_aspect_ratio=decrease", "-c:v", "png", "-f", "image2pipe", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn:{e}"))?;
    sys::adopt_child(child.id());
    let mut buf = vec![];
    let mut out = child.stdout.take().unwrap();
    let read = tokio::time::timeout(Duration::from_secs(30), out.read_to_end(&mut buf)).await;
    let _ = child.kill().await;
    match read {
        Ok(Ok(_)) if buf.len() > 8 => Ok(buf),
        _ => Err("thumb".into()),
    }
}

/// Primeira passagem do loudnorm: mede o volume do áudio.
async fn measure_loudness(
    info: &MediaInfo,
    s: &RenderSettings,
    cancel: &Cancel,
    mut on: impl FnMut(f64),
) -> Result<Option<Loudness>, FfError> {
    let Some(ai) = info.audio_index else { return Ok(None) };
    let af = format!("loudnorm=I={:.1}:TP=-1.0:LRA=11:print_format=json", s.lufs.clamp(-36.0, -5.0));
    let args: Vec<String> = [
        "-hide_banner", "-nostdin", "-v", "info", "-progress", "pipe:1", "-nostats", "-i", &info.path, "-map",
        &format!("0:{ai}"), "-vn", "-af", &af, "-f", "null", "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let dur = info.duration.max(0.001);
    let lines = run_ff(&args, s.low_priority, cancel, |p| on((p.out_time / dur).min(1.0))).await?;
    Ok(parse_loudnorm(&lines.join("\n")))
}

pub fn parse_loudnorm(txt: &str) -> Option<Loudness> {
    let start = txt.rfind('{')?;
    let end = txt[start..].find('}')? + start;
    let v: Value = serde_json::from_str(&txt[start..=end]).ok()?;
    let g = |k: &str| v[k].as_str().map(|s| s.trim().to_string());
    let l = Loudness { i: g("input_i")?, tp: g("input_tp")?, lra: g("input_lra")?, thresh: g("input_thresh")?, offset: g("target_offset")? };
    // silêncio: nada para normalizar
    let ok = |s: &str| s.parse::<f64>().map(|f| f.is_finite()).unwrap_or(false);
    if !(ok(&l.i) && ok(&l.tp) && ok(&l.lra) && ok(&l.thresh) && ok(&l.offset)) {
        return None;
    }
    Some(l)
}

// ---------------------------------------------------------------- render

#[derive(Serialize, Clone)]
struct JobEvent {
    id: String,
    phase: &'static str,
    pct: f64,
    fps: f64,
    speed: f64,
    eta: f64,
}

/// Para onde vai o progresso (a janela, ou nada nos testes).
pub type Emit = Arc<dyn Fn(&str, &'static str, f64, f64, f64, f64) + Send + Sync>;

fn emit(ev: &Emit, id: &str, phase: &'static str, pct: f64, fps: f64, speed: f64, eta: f64) {
    ev(id, phase, pct, fps, speed, eta);
}

fn window_emitter(app: AppHandle) -> Emit {
    Arc::new(move |id, phase, pct, fps, speed, eta| {
        let _ = app.emit("renderpro://job", JobEvent { id: id.into(), phase, pct, fps, speed, eta });
    })
}

#[derive(Serialize, Clone, Debug)]
pub struct JobResult {
    pub output: String,
    pub encoder: Encoder,
    pub size: u64,
    pub seconds: f64,
    pub verified: bool,
    pub warnings: Vec<Note>,
}

fn temp_path(final_path: &Path) -> PathBuf {
    let name = final_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    final_path.with_file_name(format!("{name}.renderpro-part"))
}

fn gb(b: u64) -> String {
    format!("{:.1}", b as f64 / 1_073_741_824.0)
}

/// Planeia (sem correr nada): usado pela janela de definições para mostrar o resultado.
pub fn plan_for(info: &MediaInfo, s: &RenderSettings) -> Plan {
    build_plan(info, s, &|p: &Path| p.exists())
}

pub async fn render_job(app: AppHandle, id: String, path: String, s: RenderSettings) -> Result<JobResult, String> {
    render_with(window_emitter(app), id, path, s).await
}

pub async fn render_with(ev: Emit, id: String, path: String, s: RenderSettings) -> Result<JobResult, String> {
    if s.gpu {
        detect_gpu().await;
    }
    let cancel = register(&id);
    let _active = sys::ActiveGuard::new();
    let started = Instant::now();
    let r = render_inner(&ev, &id, &path, &s, &cancel).await;
    unregister(&id);
    match &r {
        Ok(res) => log(&format!(
            "OK {path} -> {} ({} MB, {:.1}s)",
            res.output,
            res.size / 1_048_576,
            started.elapsed().as_secs_f64()
        )),
        Err(e) => log(&format!("ERRO {path}: {e}")),
    }
    r.map(|mut res| {
        res.seconds = started.elapsed().as_secs_f64();
        res
    })
}

async fn render_inner(app: &Emit, id: &str, path: &str, s: &RenderSettings, cancel: &Cancel) -> Result<JobResult, String> {
    emit(app, id, "analyze", 0.0, 0.0, 0.0, 0.0);
    let info = probe(path).await?;
    let plan = plan_for(&info, s);
    if let Some(e) = &plan.error {
        return Err(format!("plan:{}:{}", e.key, e.args.join("|")));
    }
    let final_path = PathBuf::from(&plan.output);
    let dir = final_path.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir:{e}"))?;

    // espaço em disco (mp4 com faststart reescreve o ficheiro: precisa do dobro)
    let factor = if plan.codec.muxer() == "mp4" { 2.1 } else { 1.1 };
    let need = (plan.est_bytes as f64 * factor) as u64 + 256 * 1_048_576;
    if let Some(free) = sys::free_space(&dir) {
        if free < need {
            return Err(format!("nospace:{}:{}", gb(need), gb(free)));
        }
    }

    let mut warnings: Vec<Note> = plan.warnings.clone();

    // 1) medir volume (se pedido)
    let mut loud = None;
    if plan.audio && s.normalize {
        emit(app, id, "loudness", 0.0, 0.0, 0.0, 0.0);
        let app2 = app.clone();
        let id2 = id.to_string();
        let mut last = Instant::now();
        loud = measure_loudness(&info, s, cancel, move |f| {
            if last.elapsed() > Duration::from_millis(250) {
                emit(&app2, &id2, "loudness", f * 100.0, 0.0, 0.0, 0.0);
                last = Instant::now();
            }
        })
        .await
        .map_err(|e| e.code())?;
        if loud.is_none() {
            warnings.push(Note { key: "warn.silent".into(), args: vec![] });
        }
    }

    // 2) render para o temporário — NVENC primeiro (se escolhido e disponível), CPU como rede de segurança
    let temp = temp_path(&final_path);
    let cleanup = |t: &Path| {
        let _ = std::fs::remove_file(t);
        pending_edit(t, false);
    };
    let mut encoders = vec![];
    if s.gpu && nvenc_fits(&plan) && nvenc_for(plan.codec) {
        encoders.push(Encoder::Nvenc);
    }
    encoders.push(Encoder::Cpu);

    let mut used = Encoder::Cpu;
    let mut last_err = String::new();
    let mut ok = false;
    for enc in encoders {
        let _ = std::fs::remove_file(&temp);
        pending_edit(&temp, true);
        let args = ffmpeg_args_with(&info, s, &plan, loud.as_ref(), &temp.to_string_lossy(), enc);
        log(&format!("Render {path} -> {} [{} {:?}] args: {}", plan.output, plan.codec_label, enc, args.join(" ")));

        emit(app, id, "encode", 0.0, 0.0, 0.0, 0.0);
        let dur = plan.duration.max(0.001);
        let t0 = Instant::now();
        let mut last = Instant::now();
        let app2 = app.clone();
        let id2 = id.to_string();
        let res = run_ff(&args, s.low_priority, cancel, move |p| {
            if last.elapsed() > Duration::from_millis(250) || p.end {
                let f = (p.out_time / dur).clamp(0.0, 1.0);
                let el = t0.elapsed().as_secs_f64();
                let eta = if f > 0.01 { el / f - el } else { 0.0 };
                emit(&app2, &id2, "encode", f * 100.0, p.fps, p.speed, eta);
                last = Instant::now();
            }
        })
        .await;
        let enc_lines = match res {
            Ok(l) => l,
            Err(FfError::Cancelled) => {
                cleanup(&temp);
                return Err("cancelled".into());
            }
            Err(e) => {
                cleanup(&temp);
                last_err = e.code();
                if enc == Encoder::Nvenc {
                    log(&format!("NVENC falhou ({last_err}); a repetir com a CPU"));
                    warnings.push(Note { key: "warn.nvenc_fallback".into(), args: vec![] });
                    continue;
                }
                return Err(last_err);
            }
        };

        // 3) verificação
        emit(app, id, "verify", 0.0, 0.0, 0.0, 0.0);
        if let Err(e) = verify(app, id, &temp, &plan, s, cancel).await {
            cleanup(&temp);
            if e != "cancelled" && enc == Encoder::Nvenc {
                log(&format!("Ficheiro do NVENC não passou na verificação ({e}); a repetir com a CPU"));
                warnings.push(Note { key: "warn.nvenc_fallback".into(), args: vec![] });
                last_err = e;
                continue;
            }
            return Err(e);
        }
        // linhas de erro que não sejam do próprio NVENC = problemas no original
        if let Some(l) = enc_lines.iter().find(|l| !l.to_lowercase().contains("nvenc")) {
            warnings.push(Note { key: "warn.source_errors".into(), args: vec![l.chars().take(160).collect()] });
        }
        used = enc;
        ok = true;
        break;
    }
    if !ok {
        return Err(last_err);
    }

    // 4) nome final
    let mut out = final_path.clone();
    if out.exists() && !s.overwrite {
        // apareceu entretanto um ficheiro com o mesmo nome
        out = crate::presets::output_path(Path::new(path), s, plan.codec, &|p: &Path| p.exists());
    }
    if let Err(e) = std::fs::rename(&temp, &out) {
        // destino aberto noutro programa (ex.: no player): guardar com outro nome
        let alt = crate::presets::output_path(
            Path::new(path),
            &RenderSettings { overwrite: false, ..s.clone() },
            plan.codec,
            &|p: &Path| p.exists(),
        );
        if std::fs::rename(&temp, &alt).is_err() {
            cleanup(&temp);
            return Err(format!("rename:{e}"));
        }
        warnings.push(Note { key: "warn.renamed".into(), args: vec![out.to_string_lossy().into_owned()] });
        out = alt;
    }
    pending_edit(&temp, false);
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    emit(app, id, "done", 100.0, 0.0, 0.0, 0.0);
    Ok(JobResult { output: out.to_string_lossy().into_owned(), encoder: used, size, seconds: 0.0, verified: s.verify, warnings })
}

/// Confirma que o ficheiro produzido é exatamente o que foi pedido.
async fn verify(app: &Emit, id: &str, temp: &Path, plan: &Plan, s: &RenderSettings, cancel: &Cancel) -> Result<(), String> {
    let t = temp.to_string_lossy().to_string();
    let out = probe(&t).await.map_err(|e| format!("verify:probe:{e}"))?;
    check_output(&out, plan)?;

    if s.verify {
        let args: Vec<String> = [
            "-hide_banner", "-nostdin", "-v", "error", "-xerror", "-progress", "pipe:1", "-nostats", "-i", &t, "-map",
            "0:v:0", "-map", "0:a?", "-f", "null", "-",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let dur = plan.duration.max(0.001);
        let mut last = Instant::now();
        let app2 = app.clone();
        let id2 = id.to_string();
        let t0 = Instant::now();
        let lines = run_ff(&args, s.low_priority, cancel, move |p| {
            if last.elapsed() > Duration::from_millis(250) {
                let f = (p.out_time / dur).clamp(0.0, 1.0);
                let el = t0.elapsed().as_secs_f64();
                let eta = if f > 0.01 { el / f - el } else { 0.0 };
                emit(&app2, &id2, "verify", f * 100.0, p.fps, p.speed, eta);
                last = Instant::now();
            }
        })
        .await
        .map_err(|e| match e {
            FfError::Cancelled => "cancelled".to_string(),
            FfError::Failed(_, l) => format!("verify:decode:{}", l.last().cloned().unwrap_or_default()),
            other => other.code(),
        })?;
        if let Some(l) = lines.first() {
            return Err(format!("verify:decode:{l}"));
        }
    }
    Ok(())
}

/// Comparação pura entre o que o ffprobe vê e o plano (testada).
pub fn check_output(out: &MediaInfo, plan: &Plan) -> Result<(), String> {
    if out.codec != plan.codec.probe_name() {
        return Err(format!("verify:codec:{}:{}", out.codec, plan.codec.probe_name()));
    }
    if let Some(tag) = plan.codec.hap_tag() {
        if out.codec_tag != tag {
            return Err(format!("verify:codec:{}:{}", out.codec_tag, tag));
        }
    }
    if (out.width, out.height) != (plan.out_w, plan.out_h) {
        return Err(format!("verify:size:{}x{}:{}x{}", out.width, out.height, plan.out_w, plan.out_h));
    }
    if plan.audio && out.audio_index.is_none() {
        return Err("verify:noaudio".into());
    }
    if plan.duration > 0.0 {
        let tol = (plan.duration * 0.02).max(0.5);
        if out.duration + tol < plan.duration {
            return Err(format!("verify:short:{:.2}:{:.2}", out.duration, plan.duration));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{Codec, Player};

    #[test]
    fn progress_lines() {
        let mut p = Progress::default();
        for l in ["frame=120", "fps=59.9", "out_time_us=4000000", "speed=2.5x"] {
            assert!(!parse_progress_line(l, &mut p));
        }
        assert!(parse_progress_line("progress=continue", &mut p));
        assert_eq!(p, Progress { frame: 120, out_time: 4.0, fps: 59.9, speed: 2.5, end: false });
        parse_progress_line("out_time_us=N/A", &mut p);
        assert_eq!(p.out_time, 4.0);
        assert!(parse_progress_line("progress=end", &mut p));
        assert!(p.end);
    }

    #[test]
    fn loudnorm_json() {
        let txt = "[Parsed_loudnorm_0 @ 0x1]\n{\n\t\"input_i\" : \"-27.61\",\n\t\"input_tp\" : \"-4.47\",\n\t\"input_lra\" : \"18.06\",\n\t\"input_thresh\" : \"-39.20\",\n\t\"output_i\" : \"-16.58\",\n\t\"target_offset\" : \"0.58\"\n}\n";
        let l = parse_loudnorm(txt).unwrap();
        assert_eq!(l.i, "-27.61");
        assert_eq!(l.offset, "0.58");
        let silent = txt.replace("-27.61", "-inf");
        assert!(parse_loudnorm(&silent).is_none());
    }

    fn plan(codec: Codec) -> Plan {
        let info = MediaInfo {
            path: "a.mov".into(),
            duration: 10.0,
            width: 1920,
            height: 1080,
            fps_num: 25,
            fps_den: 1,
            sar_num: 1,
            sar_den: 1,
            audio_index: Some(1),
            ..Default::default()
        };
        let s = RenderSettings { player: Player::Custom, preset: codec.id().into(), ..Default::default() };
        build_plan(&info, &s, &|_| false)
    }

    #[test]
    fn output_checks() {
        let p = plan(Codec::HapQ);
        let mut o = MediaInfo {
            codec: "hap".into(),
            codec_tag: "HapY".into(),
            width: 1920,
            height: 1080,
            duration: 10.0,
            audio_index: Some(1),
            ..Default::default()
        };
        assert!(check_output(&o, &p).is_ok());
        o.duration = 9.6;
        assert!(check_output(&o, &p).is_ok());
        o.duration = 8.0;
        assert!(check_output(&o, &p).unwrap_err().starts_with("verify:short"));
        o.duration = 10.0;
        o.codec_tag = "Hap1".into();
        assert!(check_output(&o, &p).is_err());
        o.codec_tag = "HapY".into();
        o.audio_index = None;
        assert_eq!(check_output(&o, &p).unwrap_err(), "verify:noaudio");
    }

    #[test]
    fn temp_names() {
        let t = temp_path(Path::new("C:/a/b_HAP.mov"));
        assert!(t.to_string_lossy().ends_with("b_HAP.mov.renderpro-part"));
    }
}
