//! Codecs, presets recomendados por player e planeamento de cada render.
//!
//! Tudo aqui é puro (sem I/O) para poder ser testado: a partir do que o ffprobe
//! leu e das definições escolhidas, decide o codec, as dimensões, o nome do
//! ficheiro final, os avisos e os argumentos exatos do ffmpeg.

use crate::media::MediaInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Hap,
    HapAlpha,
    HapQ,
    H264,
    H265,
    ProresHq,
    Prores4444,
    DnxhrHq,
}

pub const ALL_CODECS: &[Codec] = &[
    Codec::Hap,
    Codec::HapAlpha,
    Codec::HapQ,
    Codec::H264,
    Codec::H265,
    Codec::ProresHq,
    Codec::Prores4444,
    Codec::DnxhrHq,
];

impl Codec {
    pub fn id(self) -> &'static str {
        match self {
            Codec::Hap => "hap",
            Codec::HapAlpha => "hap_alpha",
            Codec::HapQ => "hap_q",
            Codec::H264 => "h264",
            Codec::H265 => "h265",
            Codec::ProresHq => "prores_hq",
            Codec::Prores4444 => "prores_4444",
            Codec::DnxhrHq => "dnxhr_hq",
        }
    }
    pub fn from_id(s: &str) -> Option<Codec> {
        ALL_CODECS.iter().copied().find(|c| c.id() == s)
    }
    pub fn label(self) -> &'static str {
        match self {
            Codec::Hap => "HAP",
            Codec::HapAlpha => "HAP Alpha",
            Codec::HapQ => "HAP Q",
            Codec::H264 => "H.264",
            Codec::H265 => "H.265",
            Codec::ProresHq => "ProRes 422 HQ",
            Codec::Prores4444 => "ProRes 4444",
            Codec::DnxhrHq => "DNxHR HQ",
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Codec::Hap => "HAP",
            Codec::HapAlpha => "HAPALPHA",
            Codec::HapQ => "HAPQ",
            Codec::H264 => "H264",
            Codec::H265 => "H265",
            Codec::ProresHq => "PRORESHQ",
            Codec::Prores4444 => "PRORES4444",
            Codec::DnxhrHq => "DNXHRHQ",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Codec::H264 | Codec::H265 => "mp4",
            _ => "mov",
        }
    }
    pub fn muxer(self) -> &'static str {
        match self {
            Codec::H264 | Codec::H265 => "mp4",
            _ => "mov",
        }
    }
    pub fn is_hap(self) -> bool {
        matches!(self, Codec::Hap | Codec::HapAlpha | Codec::HapQ)
    }
    pub fn alpha(self) -> bool {
        matches!(self, Codec::HapAlpha | Codec::Prores4444)
    }
    /// Múltiplo exigido para largura/altura.
    pub fn align(self) -> (u32, u32) {
        match self {
            // texturas DXT/BC: blocos de 4×4 (o encoder recusa outros tamanhos e o D3D11 também)
            Codec::Hap | Codec::HapAlpha | Codec::HapQ => (4, 4),
            // 4:2:0
            Codec::H264 | Codec::H265 => (2, 2),
            // 4:2:2
            Codec::ProresHq | Codec::DnxhrHq => (2, 1),
            Codec::Prores4444 => (1, 1),
        }
    }
    pub fn pix_fmt(self, alpha: bool) -> &'static str {
        match self {
            Codec::Hap | Codec::HapAlpha | Codec::HapQ => "rgba",
            Codec::H264 | Codec::H265 => "yuv420p",
            Codec::ProresHq => "yuv422p10le",
            Codec::Prores4444 => {
                if alpha {
                    "yuva444p10le"
                } else {
                    "yuv444p10le"
                }
            }
            Codec::DnxhrHq => "yuv422p",
        }
    }
    /// Nome do codec que o ffprobe tem de ver no ficheiro final.
    pub fn probe_name(self) -> &'static str {
        match self {
            Codec::Hap | Codec::HapAlpha | Codec::HapQ => "hap",
            Codec::H264 => "h264",
            Codec::H265 => "hevc",
            Codec::ProresHq | Codec::Prores4444 => "prores",
            Codec::DnxhrHq => "dnxhd",
        }
    }
    pub fn hap_tag(self) -> Option<&'static str> {
        match self {
            Codec::Hap => Some("Hap1"),
            Codec::HapAlpha => Some("Hap5"),
            Codec::HapQ => Some("HapY"),
            _ => None,
        }
    }
    /// Bytes por píxel por frame (estimativa por excesso, para verificar o espaço em disco).
    pub fn bytes_per_px(self) -> f64 {
        match self {
            Codec::Hap => 0.5,
            Codec::HapAlpha | Codec::HapQ => 1.0,
            Codec::H264 => 0.15,
            Codec::H265 => 0.1,
            Codec::ProresHq | Codec::DnxhrHq => 0.5,
            Codec::Prores4444 => 0.9,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Player {
    #[default]
    Playerslice,
    Videohost,
    Custom,
}

/// Limites de cada player (o que garante que o vídeo toca).
pub fn max_dim(player: Player, codec: Codec) -> u32 {
    match (player, codec) {
        // NVDEC/D3D11VA: H.264 até 4096, H.265 até 8192
        (Player::Playerslice, Codec::H264) => 4096,
        (Player::Playerslice, Codec::H265) => 8192,
        // texturas Direct3D 11
        (_, c) if c.is_hap() => 16384,
        _ => 16384,
    }
}

/// Codecs que cada player toca (por ordem de preferência).
pub fn player_codecs(player: Player) -> &'static [Codec] {
    match player {
        Player::Playerslice => &[
            Codec::Hap,
            Codec::HapQ,
            Codec::HapAlpha,
            Codec::H264,
            Codec::H265,
            Codec::ProresHq,
            Codec::Prores4444,
            Codec::DnxhrHq,
        ],
        Player::Videohost => &[Codec::H264, Codec::Hap, Codec::HapQ, Codec::HapAlpha, Codec::ProresHq, Codec::H265],
        Player::Custom => ALL_CODECS,
    }
}

/// Uma mensagem para a interface: chave de tradução + valores.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Note {
    pub key: String,
    pub args: Vec<String>,
}

fn note(key: &str, args: &[&str]) -> Note {
    Note { key: key.into(), args: args.iter().map(|s| s.to_string()).collect() }
}

/// O codec recomendado para este vídeo neste player, e porquê.
pub fn recommend(info: &MediaInfo, player: Player) -> (Codec, Note) {
    let (w, h) = info.display_size();
    let px = w as f64 * h as f64;
    let rate = px * info.fps().max(1.0);
    match player {
        Player::Playerslice | Player::Custom => {
            if info.has_alpha {
                (Codec::HapAlpha, note("why.ps_alpha", &[]))
            } else {
                (Codec::Hap, note("why.ps_hap", &[]))
            }
        }
        Player::Videohost => {
            if info.has_alpha {
                (Codec::HapAlpha, note("why.vh_alpha", &[]))
            } else if rate > 1920.0 * 1080.0 * 60.0 * 1.05 {
                (Codec::Hap, note("why.vh_heavy", &[]))
            } else {
                (Codec::H264, note("why.vh_h264", &[]))
            }
        }
    }
}

// ---------------------------------------------------------------- definições

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SizeMode {
    #[default]
    Original,
    Half,
    Custom,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Stretch,
    Fit,
    Fill,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlignMode {
    /// Acrescenta margem preta à direita/em baixo: os píxeis ficam 1:1.
    #[default]
    Pad,
    /// Redimensiona ligeiramente.
    Scale,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Normal,
    #[default]
    High,
    Max,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Beside,
    Subfolder,
    Folder,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RenderSettings {
    pub player: Player,
    /// "auto" (recomendado para cada vídeo) ou o id de um codec.
    pub preset: String,
    pub quality: Quality,
    pub size: SizeMode,
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
    pub align: AlignMode,
    /// None = frame rate original (sempre constante na saída).
    pub fps: Option<String>,
    pub audio: bool,
    pub stereo: bool,
    pub normalize: bool,
    pub lufs: f64,
    pub deinterlace: bool,
    pub output: OutputMode,
    pub folder: String,
    pub suffix: bool,
    pub overwrite: bool,
    pub verify: bool,
    pub low_priority: bool,
    /// H.264/H.265 pela gráfica NVIDIA (NVENC) quando existir; se falhar volta ao x264/x265.
    pub gpu: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            player: Player::Playerslice,
            preset: "auto".into(),
            quality: Quality::High,
            size: SizeMode::Original,
            width: 1920,
            height: 1080,
            fit: Fit::Stretch,
            align: AlignMode::Pad,
            fps: None,
            audio: true,
            stereo: true,
            normalize: false,
            lufs: -16.0,
            deinterlace: true,
            output: OutputMode::Beside,
            folder: String::new(),
            suffix: true,
            overwrite: false,
            verify: true,
            low_priority: true,
            gpu: true,
        }
    }
}

// ---------------------------------------------------------------- plano

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    pub codec: Codec,
    pub codec_label: String,
    pub recommended: Codec,
    pub why: Note,
    pub auto: bool,
    pub out_w: u32,
    pub out_h: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub duration: f64,
    pub frames: u64,
    pub audio: bool,
    pub output: String,
    pub est_bytes: u64,
    pub chunks: u32,
    pub warnings: Vec<Note>,
    /// Se existir, este vídeo não pode ser feito com estas definições.
    pub error: Option<Note>,
}

fn round_up(v: u32, m: u32) -> u32 {
    if m <= 1 {
        v.max(1)
    } else {
        v.max(1).div_ceil(m) * m
    }
}

fn round_near(v: u32, m: u32) -> u32 {
    if m <= 1 {
        return v.max(1);
    }
    let r = ((v as f64 / m as f64).round() as u32).max(1) * m;
    r.max(m)
}

/// Número de chunks HAP: o PLAYERSLICE descomprime os chunks em paralelo.
/// Recomendação do PLAYERSLICE: 4 a 8 em resoluções grandes.
pub fn hap_chunks(w: u32, h: u32) -> u32 {
    let px = w as f64 * h as f64;
    let hd = 1920.0 * 1080.0;
    if px > hd * 2.0 {
        8
    } else if px >= hd * 0.9 {
        4
    } else {
        ((px / (hd / 4.0)).ceil() as u32).clamp(1, 4)
    }
}

fn parse_fps(s: &str) -> Option<(u32, u32)> {
    if let Some(r) = crate::media::parse_ratio(s) {
        return Some(r);
    }
    let f: f64 = s.trim().replace(',', ".").parse().ok()?;
    if !(1.0..=240.0).contains(&f) {
        return None;
    }
    // 29.97 / 59.94 / 23.976 → frações NTSC exatas
    for (n, d) in [(24000u32, 1001u32), (30000, 1001), (60000, 1001)] {
        if (f - n as f64 / d as f64).abs() < 0.01 {
            return Some((n, d));
        }
    }
    if (f - f.round()).abs() < 1e-6 {
        return Some((f.round() as u32, 1));
    }
    Some(((f * 1000.0).round() as u32, 1000))
}

fn clean_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into())
}

pub fn output_dir(input: &Path, s: &RenderSettings) -> PathBuf {
    let parent = input.parent().map(Path::to_path_buf).unwrap_or_default();
    match s.output {
        OutputMode::Beside => parent,
        OutputMode::Subfolder => parent.join("RenderPRO"),
        OutputMode::Folder if !s.folder.trim().is_empty() => PathBuf::from(s.folder.trim()),
        OutputMode::Folder => parent,
    }
}

/// Caminho final. Nunca devolve o próprio original; sem "substituir" escolhe um nome livre.
pub fn output_path(input: &Path, s: &RenderSettings, codec: Codec, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let dir = output_dir(input, s);
    let stem = clean_stem(input);
    let base = if s.suffix { format!("{stem}_{}", codec.suffix()) } else { stem.clone() };
    let same = |p: &Path| -> bool {
        let a = p.to_string_lossy().to_lowercase().replace('\\', "/");
        let b = input.to_string_lossy().to_lowercase().replace('\\', "/");
        a == b
    };
    let mut candidate = dir.join(format!("{base}.{}", codec.ext()));
    if same(&candidate) {
        candidate = dir.join(format!("{stem}_{}.{}", codec.suffix(), codec.ext()));
    }
    if s.overwrite && !same(&candidate) {
        return candidate;
    }
    let first_base = candidate.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(base);
    let mut n = 2;
    while exists(&candidate) || same(&candidate) {
        candidate = dir.join(format!("{first_base} ({n}).{}", codec.ext()));
        n += 1;
    }
    candidate
}

pub fn chosen_codec(info: &MediaInfo, s: &RenderSettings) -> (Codec, Codec, Note, bool) {
    let (rec, why) = recommend(info, s.player);
    match Codec::from_id(&s.preset) {
        Some(c) => (c, rec, why, false),
        None => (rec, rec, why, true),
    }
}

pub fn build_plan(info: &MediaInfo, s: &RenderSettings, exists: &dyn Fn(&Path) -> bool) -> Plan {
    let (codec, recommended, why, auto) = chosen_codec(info, s);
    let mut warnings = vec![];
    let mut error = None;

    // ---- tamanho
    let (sw, sh) = info.display_size();
    let (tw, th) = match s.size {
        SizeMode::Original => (sw, sh),
        SizeMode::Half => ((sw / 2).max(1), (sh / 2).max(1)),
        SizeMode::Custom => (s.width.clamp(16, 16384), s.height.clamp(16, 16384)),
    };
    let (ax, ay) = codec.align();
    let (out_w, out_h) = match s.align {
        AlignMode::Pad => (round_up(tw, ax), round_up(th, ay)),
        AlignMode::Scale => (round_near(tw, ax), round_near(th, ay)),
    };
    if (out_w, out_h) != (tw, th) {
        let key = if s.align == AlignMode::Pad { "warn.pad" } else { "warn.scale_align" };
        warnings.push(note(key, &[&format!("{tw}×{th}"), &format!("{out_w}×{out_h}"), codec.label()]));
    }
    let limit = max_dim(s.player, codec);
    if out_w > limit || out_h > limit {
        error = Some(note("err.too_big", &[codec.label(), &limit.to_string()]));
    }

    // ---- alfa
    if info.has_alpha && !codec.alpha() {
        warnings.push(note("warn.alpha_lost", &[codec.label()]));
    }
    if codec.alpha() && !info.has_alpha {
        warnings.push(note("warn.alpha_useless", &[codec.label()]));
    }

    // ---- frame rate
    let (fps_num, fps_den) = s
        .fps
        .as_deref()
        .and_then(parse_fps)
        .unwrap_or((info.fps_num.max(1), info.fps_den.max(1)));
    if info.vfr && s.fps.is_none() {
        warnings.push(note("warn.vfr", &[&format!("{:.3}", fps_num as f64 / fps_den as f64)]));
    }
    if info.interlaced {
        warnings.push(note(if s.deinterlace { "warn.deinterlaced" } else { "warn.interlaced" }, &[]));
    }
    if info.hdr {
        warnings.push(note("warn.hdr", &[]));
    }
    if info.rotation != 0 {
        warnings.push(note("warn.rotated", &[&info.rotation.to_string()]));
    }
    if s.player == Player::Videohost && codec == Codec::H265 {
        warnings.push(note("warn.vh_h265", &[]));
    }
    if s.player == Player::Videohost && matches!(codec, Codec::ProresHq) && (out_w as u64 * out_h as u64) > 1920 * 1080 {
        warnings.push(note("warn.vh_prores_big", &[]));
    }
    if s.player == Player::Playerslice && matches!(codec, Codec::H264 | Codec::H265) {
        warnings.push(note("warn.ps_longgop", &[codec.label()]));
    }
    if s.player == Player::Playerslice && matches!(codec, Codec::ProresHq | Codec::Prores4444 | Codec::DnxhrHq) {
        warnings.push(note("warn.ps_cpu", &[codec.label()]));
    }

    let audio = s.audio && info.has_audio();
    if info.audio_streams > 1 && audio {
        warnings.push(note("warn.multi_audio", &[&info.audio_streams.to_string()]));
    }
    if info.duration <= 0.0 {
        warnings.push(note("warn.no_duration", &[]));
    }

    let fps = fps_num as f64 / fps_den as f64;
    let frames = if info.duration > 0.0 { (info.duration * fps).round() as u64 } else { 0 };
    let video_bytes = codec.bytes_per_px() * out_w as f64 * out_h as f64 * frames as f64;
    let audio_bytes = if audio { info.duration * 48_000.0 * 2.0 * 3.0 } else { 0.0 };
    let est_bytes = (video_bytes + audio_bytes) as u64;

    let output = output_path(Path::new(&info.path), s, codec, exists);
    Plan {
        codec,
        codec_label: codec.label().into(),
        recommended,
        why,
        auto,
        out_w,
        out_h,
        fps_num,
        fps_den,
        duration: info.duration,
        frames,
        audio,
        output: output.to_string_lossy().into_owned(),
        est_bytes,
        chunks: if codec.is_hap() { hap_chunks(out_w, out_h) } else { 0 },
        warnings,
        error,
    }
}

// ---------------------------------------------------------------- ffmpeg

/// Medidas da primeira passagem do loudnorm.
#[derive(Clone, Debug, PartialEq)]
pub struct Loudness {
    pub i: String,
    pub tp: String,
    pub lra: String,
    pub thresh: String,
    pub offset: String,
}

/// Matriz de cor de entrada: a que vem no ficheiro ou, sem etiqueta, a habitual para o tamanho.
fn in_matrix(info: &MediaInfo) -> &'static str {
    match info.color_space.as_str() {
        "bt709" => "bt709",
        "bt470bg" => "bt470",
        "smpte170m" => "smpte170m",
        "bt2020nc" | "bt2020c" => "bt2020",
        "smpte240m" => "smpte240m",
        "fcc" => "fcc",
        _ => {
            if info.height >= 720 || info.width >= 1280 {
                "bt709"
            } else {
                "bt601"
            }
        }
    }
}

/// Cadeia de filtros de vídeo.
pub fn video_filter(info: &MediaInfo, s: &RenderSettings, p: &Plan) -> String {
    let mut f: Vec<String> = vec![];
    if info.interlaced && s.deinterlace {
        f.push("bwdif=mode=send_frame:parity=auto:deint=interlaced".into());
    }
    f.push(format!("fps={}/{}", p.fps_num, p.fps_den));

    let (sw, sh) = info.display_size();
    let (tw, th) = match s.size {
        SizeMode::Original => (sw, sh),
        SizeMode::Half => ((sw / 2).max(1), (sh / 2).max(1)),
        SizeMode::Custom => (s.width.clamp(16, 16384), s.height.clamp(16, 16384)),
    };
    let scale_to_final = s.align == AlignMode::Scale;
    let (sx, sy) = if scale_to_final { (p.out_w, p.out_h) } else { (tw, th) };

    let rgb_in = !info.pix_fmt.starts_with("yuv") && !info.pix_fmt.starts_with("nv") && !info.pix_fmt.starts_with("p0");
    let mut color = String::new();
    if !rgb_in {
        color.push_str(&format!(":in_color_matrix={}", in_matrix(info)));
        let r = match info.color_range.as_str() {
            "pc" | "jpeg" => "full",
            "tv" | "mpeg" => "limited",
            _ => "auto",
        };
        color.push_str(&format!(":in_range={r}"));
    }
    let yuv_out = !p.codec.is_hap();
    if yuv_out {
        color.push_str(":out_color_matrix=bt709:out_range=limited");
    }

    let custom = s.size == SizeMode::Custom;
    match (custom, s.fit) {
        (true, Fit::Fit) => {
            f.push(format!("scale={sx}:{sy}:force_original_aspect_ratio=decrease:flags=lanczos{color}"));
            f.push(format!("format={}", p.codec.pix_fmt(p.codec.alpha() && info.has_alpha)));
            let pad_color = if p.codec.alpha() { "black@0" } else { "black" };
            f.push(format!("pad={sx}:{sy}:(ow-iw)/2:(oh-ih)/2:color={pad_color}"));
        }
        (true, Fit::Fill) => {
            f.push(format!("scale={sx}:{sy}:force_original_aspect_ratio=increase:flags=lanczos{color}"));
            f.push(format!("format={}", p.codec.pix_fmt(p.codec.alpha() && info.has_alpha)));
            f.push(format!("crop={sx}:{sy}"));
        }
        _ => {
            f.push(format!("scale={sx}:{sy}:flags=lanczos{color}"));
            f.push(format!("format={}", p.codec.pix_fmt(p.codec.alpha() && info.has_alpha)));
        }
    }
    if !scale_to_final && (p.out_w, p.out_h) != (tw, th) {
        let pad_color = if p.codec.alpha() { "black@0" } else { "black" };
        f.push(format!("pad={}:{}:0:0:color={pad_color}", p.out_w, p.out_h));
    }
    f.push("setsar=1".into());
    f.join(",")
}

pub fn audio_filter(s: &RenderSettings, loud: Option<&Loudness>) -> String {
    let mut f = vec!["aresample=async=1:first_pts=0".to_string()];
    if let Some(l) = loud {
        f.push(format!(
            "loudnorm=I={:.1}:TP=-1.0:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
            s.lufs.clamp(-36.0, -5.0),
            l.i,
            l.tp,
            l.lra,
            l.thresh,
            l.offset
        ));
    }
    f.push("aresample=48000".into());
    f.join(",")
}

fn crf(codec: Codec, q: Quality) -> u32 {
    match (codec, q) {
        (Codec::H264, Quality::Normal) => 20,
        (Codec::H264, Quality::High) => 16,
        (Codec::H264, Quality::Max) => 12,
        (_, Quality::Normal) => 23,
        (_, Quality::High) => 19,
        (_, Quality::Max) => 15,
    }
}

/// Limite de bitrate para H.264/H.265: a descodificação fica previsível (sobretudo por CPU no VIDEOHOST).
pub fn max_rate_kbps(w: u32, h: u32, fps: f64, q: Quality) -> u32 {
    let base = 50_000.0 * (w as f64 * h as f64 * fps.max(1.0)) / (1920.0 * 1080.0 * 30.0);
    let mult = match q {
        Quality::Normal => 0.7,
        Quality::High => 1.0,
        Quality::Max => 1.4,
    };
    (base * mult).clamp(8_000.0, 240_000.0) as u32
}

/// Quem codifica o vídeo H.264/H.265.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoder {
    /// x264 / x265 (CPU) — referência, sempre disponível.
    Cpu,
    /// NVENC (gráfica NVIDIA).
    Nvenc,
}

/// O NVENC serve para este plano? (limites de tamanho do hardware)
pub fn nvenc_fits(p: &Plan) -> bool {
    let (max, min_w, min_h) = match p.codec {
        Codec::H264 => (4096, 160, 64),
        Codec::H265 => (8192, 160, 64),
        _ => return false,
    };
    p.out_w <= max && p.out_h <= max && p.out_w >= min_w && p.out_h >= min_h
}

fn nvenc_cq(codec: Codec, q: Quality) -> u32 {
    // o CQ do NVENC dá ficheiros parecidos com CRF ~3 abaixo
    match (codec, q) {
        (Codec::H264, Quality::Normal) => 23,
        (Codec::H264, Quality::High) => 19,
        (Codec::H264, Quality::Max) => 15,
        (_, Quality::Normal) => 26,
        (_, Quality::High) => 22,
        (_, Quality::Max) => 18,
    }
}

/// Argumentos completos do ffmpeg para o render (escreve em `temp`).
#[cfg(test)]
pub fn ffmpeg_args(info: &MediaInfo, s: &RenderSettings, p: &Plan, loud: Option<&Loudness>, temp: &str) -> Vec<String> {
    ffmpeg_args_with(info, s, p, loud, temp, Encoder::Cpu)
}

pub fn ffmpeg_args_with(
    info: &MediaInfo,
    s: &RenderSettings,
    p: &Plan,
    loud: Option<&Loudness>,
    temp: &str,
    enc: Encoder,
) -> Vec<String> {
    let mut a: Vec<String> = vec![];
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    push(&["-hide_banner", "-nostdin", "-y", "-v", "error", "-progress", "pipe:1", "-nostats"]);
    push(&["-i", &info.path]);
    let vmap = format!("0:{}", info.video_index.unwrap_or(0));
    push(&["-map", &vmap]);
    if p.audio {
        if let Some(ai) = info.audio_index {
            let amap = format!("0:{ai}");
            push(&["-map", &amap]);
        }
    }
    push(&["-sn", "-dn", "-map_metadata", "-1", "-map_chapters", "-1"]);
    let vf = video_filter(info, s, p);
    push(&["-vf", &vf]);

    let g = ((p.fps_num as f64 / p.fps_den as f64).round() as u32).max(1).to_string();
    let fps = p.fps_num as f64 / p.fps_den as f64;
    match p.codec {
        Codec::Hap | Codec::HapAlpha | Codec::HapQ => {
            let fmt = match p.codec {
                Codec::Hap => "hap",
                Codec::HapAlpha => "hap_alpha",
                _ => "hap_q",
            };
            let chunks = p.chunks.max(1).to_string();
            push(&["-c:v", "hap", "-format", fmt, "-compressor", "snappy", "-chunks", &chunks]);
        }
        Codec::H264 | Codec::H265 if enc == Encoder::Nvenc => {
            let cq = nvenc_cq(p.codec, s.quality).to_string();
            let mut mr = max_rate_kbps(p.out_w, p.out_h, fps, s.quality);
            if p.codec == Codec::H265 {
                mr = mr * 7 / 10;
            }
            let (mr, bs) = (format!("{mr}k"), format!("{}k", mr * 2));
            let (name, profile) = if p.codec == Codec::H264 { ("h264_nvenc", "high") } else { ("hevc_nvenc", "main") };
            push(&[
                "-c:v", name, "-preset", "p6", "-tune", "hq", "-rc", "vbr", "-cq", &cq, "-b:v", "0", "-maxrate", &mr,
                "-bufsize", &bs, "-profile:v", profile, "-pix_fmt", "yuv420p", "-g", &g, "-bf", "2", "-strict_gop", "1",
                "-no-scenecut", "1", "-forced-idr", "1", "-spatial-aq", "1", "-rc-lookahead", "20",
            ]);
            if p.codec == Codec::H265 {
                push(&["-tag:v", "hvc1"]);
            }
        }
        Codec::H264 => {
            let c = crf(Codec::H264, s.quality).to_string();
            let mr = max_rate_kbps(p.out_w, p.out_h, fps, s.quality);
            let (mr, bs) = (format!("{mr}k"), format!("{}k", mr * 2));
            push(&[
                "-c:v", "libx264", "-preset", "slow", "-crf", &c, "-maxrate", &mr, "-bufsize", &bs, "-profile:v", "high",
                "-pix_fmt", "yuv420p", "-g", &g, "-keyint_min", &g, "-sc_threshold", "0", "-x264-params", "open-gop=0",
            ]);
        }
        Codec::H265 => {
            let c = crf(Codec::H265, s.quality).to_string();
            let mr = max_rate_kbps(p.out_w, p.out_h, fps, s.quality) * 7 / 10;
            let params = format!(
                "keyint={g}:min-keyint={g}:scenecut=0:open-gop=0:vbv-maxrate={mr}:vbv-bufsize={}:log-level=error",
                mr * 2
            );
            push(&[
                "-c:v", "libx265", "-preset", "medium", "-crf", &c, "-profile:v", "main", "-pix_fmt", "yuv420p", "-tag:v",
                "hvc1", "-x265-params", &params,
            ]);
        }
        Codec::ProresHq => push(&["-c:v", "prores_ks", "-profile:v", "3", "-vendor", "apl0"]),
        Codec::Prores4444 => {
            push(&["-c:v", "prores_ks", "-profile:v", "4", "-vendor", "apl0"]);
            if info.has_alpha {
                push(&["-alpha_bits", "16"]);
            }
        }
        Codec::DnxhrHq => push(&["-c:v", "dnxhd", "-profile:v", "dnxhr_hq"]),
    }
    if !p.codec.is_hap() {
        push(&["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"]);
    }

    if p.audio {
        let af = audio_filter(s, loud);
        push(&["-af", &af]);
        if s.stereo {
            push(&["-ac", "2"]);
        }
        if p.codec.muxer() == "mp4" {
            push(&["-c:a", "aac", "-b:a", "320k"]);
        } else {
            push(&["-c:a", "pcm_s16le"]);
        }
    } else {
        push(&["-an"]);
    }

    push(&["-threads", "0", "-max_muxing_queue_size", "4096", "-metadata", "encoder=RenderPRO"]);
    if p.codec.muxer() == "mp4" {
        push(&["-movflags", "+faststart"]);
    }
    push(&["-f", p.codec.muxer(), temp]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(w: u32, h: u32) -> MediaInfo {
        MediaInfo {
            path: "C:/v/clip.mp4".into(),
            duration: 10.0,
            video_index: Some(0),
            codec: "h264".into(),
            width: w,
            height: h,
            pix_fmt: "yuv420p".into(),
            fps_num: 25,
            fps_den: 1,
            sar_num: 1,
            sar_den: 1,
            audio_index: Some(1),
            audio_streams: 1,
            ..Default::default()
        }
    }
    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn recommendations() {
        let mut i = info(1920, 1080);
        assert_eq!(recommend(&i, Player::Playerslice).0, Codec::Hap);
        assert_eq!(recommend(&i, Player::Videohost).0, Codec::H264);
        i.fps_num = 60;
        assert_eq!(recommend(&i, Player::Videohost).0, Codec::H264);
        let i4k = info(3840, 2160);
        assert_eq!(recommend(&i4k, Player::Videohost).0, Codec::Hap);
        let mut a = info(1920, 1080);
        a.has_alpha = true;
        assert_eq!(recommend(&a, Player::Playerslice).0, Codec::HapAlpha);
        assert_eq!(recommend(&a, Player::Videohost).0, Codec::HapAlpha);
    }

    #[test]
    fn hap_pads_to_multiple_of_four() {
        let i = info(721, 647);
        let p = build_plan(&i, &RenderSettings::default(), &never);
        assert_eq!((p.out_w, p.out_h), (724, 648));
        assert_eq!(p.codec, Codec::Hap);
        assert!(p.warnings.iter().any(|w| w.key == "warn.pad"));
        let vf = video_filter(&i, &RenderSettings::default(), &p);
        assert!(vf.contains("scale=721:647"), "{vf}");
        assert!(vf.ends_with("pad=724:648:0:0:color=black,setsar=1"), "{vf}");
        assert!(vf.contains("format=rgba"));
    }

    #[test]
    fn scale_align_rounds_to_nearest() {
        let s = RenderSettings { align: AlignMode::Scale, ..Default::default() };
        let p = build_plan(&info(721, 646), &s, &never);
        assert_eq!((p.out_w, p.out_h), (720, 648));
        let vf = video_filter(&info(721, 646), &s, &p);
        assert!(vf.contains("scale=720:648"));
        assert!(!vf.contains("pad="));
    }

    #[test]
    fn h264_too_big_for_nvdec_is_blocked() {
        let s = RenderSettings { preset: "h264".into(), ..Default::default() };
        let p = build_plan(&info(7680, 1080), &s, &never);
        assert!(p.error.is_some());
        let s5 = RenderSettings { preset: "h265".into(), ..Default::default() };
        assert!(build_plan(&info(7680, 1080), &s5, &never).error.is_none());
        let shap = RenderSettings::default();
        assert!(build_plan(&info(7680, 1080), &shap, &never).error.is_none());
    }

    #[test]
    fn output_names_never_clash_or_touch_source() {
        let s = RenderSettings::default();
        let input = Path::new("C:/v/clip.mov");
        let taken = |p: &Path| p.to_string_lossy().ends_with("clip_HAP.mov");
        let out = output_path(input, &s, Codec::Hap, &taken);
        assert!(out.to_string_lossy().ends_with("clip_HAP (2).mov"), "{out:?}");

        let nosuf = RenderSettings { suffix: false, overwrite: true, ..Default::default() };
        let out = output_path(input, &nosuf, Codec::Hap, &never);
        assert!(out.to_string_lossy().ends_with("clip_HAP.mov"), "{out:?}");

        let sub = RenderSettings { output: OutputMode::Subfolder, ..Default::default() };
        let out = output_path(input, &sub, Codec::H264, &never);
        assert!(out.to_string_lossy().replace('\\', "/").ends_with("v/RenderPRO/clip_H264.mp4"), "{out:?}");
    }

    #[test]
    fn fps_parsing() {
        assert_eq!(parse_fps("29.97"), Some((30000, 1001)));
        assert_eq!(parse_fps("59,94"), Some((60000, 1001)));
        assert_eq!(parse_fps("25"), Some((25, 1)));
        assert_eq!(parse_fps("30000/1001"), Some((30000, 1001)));
        assert_eq!(parse_fps("0"), None);
    }

    #[test]
    fn h264_args_are_safe_for_hardware_decoders() {
        let s = RenderSettings { player: Player::Videohost, preset: "auto".into(), ..Default::default() };
        let mut i = info(1920, 1080);
        i.pix_fmt = "yuv422p10le".into();
        let p = build_plan(&i, &s, &never);
        assert_eq!(p.codec, Codec::H264);
        let a = ffmpeg_args(&i, &s, &p, None, "out.tmp").join(" ");
        assert!(a.contains("-pix_fmt yuv420p"));
        assert!(a.contains("-profile:v high"));
        assert!(a.contains("-g 25 -keyint_min 25 -sc_threshold 0"));
        assert!(a.contains("-c:a aac"));
        assert!(a.contains("-movflags +faststart"));
        assert!(a.contains("-map 0:0 -map 0:1"));
        assert!(a.ends_with("-f mp4 out.tmp"));
    }

    #[test]
    fn hap_args() {
        let s = RenderSettings::default();
        let i = info(3840, 2160);
        let p = build_plan(&i, &s, &never);
        assert_eq!(p.chunks, 8);
        let a = ffmpeg_args(&i, &s, &p, None, "o").join(" ");
        assert!(a.contains("-c:v hap -format hap -compressor snappy -chunks 8"));
        assert!(a.contains("-c:a pcm_s16le"));
        assert!(a.contains("-f mov"));
        assert_eq!(hap_chunks(1920, 1080), 4);
        assert_eq!(hap_chunks(640, 360), 1);
        assert_eq!(hap_chunks(4864, 936), 8);
        assert_eq!(hap_chunks(1280, 720), 2);
    }

    #[test]
    fn audio_removed_or_normalized() {
        let s = RenderSettings { audio: false, ..Default::default() };
        let i = info(1920, 1080);
        let p = build_plan(&i, &s, &never);
        let a = ffmpeg_args(&i, &s, &p, None, "o").join(" ");
        assert!(a.contains("-an") && !a.contains("-map 0:1"));

        let s = RenderSettings { normalize: true, lufs: -23.0, ..Default::default() };
        let p = build_plan(&i, &s, &never);
        let l = Loudness { i: "-30".into(), tp: "-6".into(), lra: "5".into(), thresh: "-40".into(), offset: "0.2".into() };
        let a = ffmpeg_args(&i, &s, &p, Some(&l), "o").join(" ");
        assert!(a.contains("loudnorm=I=-23.0"), "{a}");
        assert!(a.contains(",aresample=48000"));
    }

    #[test]
    fn nvenc_args_and_limits() {
        let s = RenderSettings { player: Player::Videohost, preset: "h264".into(), ..Default::default() };
        let i = info(1920, 1080);
        let p = build_plan(&i, &s, &never);
        assert!(nvenc_fits(&p));
        let a = ffmpeg_args_with(&i, &s, &p, None, "o", Encoder::Nvenc).join(" ");
        assert!(a.contains("-c:v h264_nvenc"), "{a}");
        assert!(a.contains("-pix_fmt yuv420p -g 25"), "{a}");
        assert!(a.contains("-no-scenecut 1"));
        assert!(!a.contains("libx264"));
        let cpu = ffmpeg_args(&i, &s, &p, None, "o").join(" ");
        assert!(cpu.contains("libx264"));

        let s5 = RenderSettings { player: Player::Custom, preset: "h265".into(), ..Default::default() };
        let p5 = build_plan(&info(7680, 1080), &s5, &never);
        assert!(nvenc_fits(&p5));
        assert!(ffmpeg_args_with(&info(7680, 1080), &s5, &p5, None, "o", Encoder::Nvenc).join(" ").contains("hevc_nvenc"));
        let s4 = RenderSettings { player: Player::Custom, preset: "h264".into(), ..Default::default() };
        assert!(!nvenc_fits(&build_plan(&info(5000, 1080), &s4, &never)));
        assert!(!nvenc_fits(&build_plan(&info(1920, 1080), &RenderSettings::default(), &never))); // HAP
    }

    #[test]
    fn custom_fit_letterboxes() {
        let s = RenderSettings { size: SizeMode::Custom, width: 1920, height: 1080, fit: Fit::Fit, ..Default::default() };
        let i = info(1080, 1920);
        let p = build_plan(&i, &s, &never);
        let vf = video_filter(&i, &s, &p);
        assert!(vf.contains("force_original_aspect_ratio=decrease"));
        assert!(vf.contains("pad=1920:1080:(ow-iw)/2:(oh-ih)/2"));
    }

    #[test]
    fn untagged_hd_uses_bt709() {
        let i = info(1920, 1080);
        let p = build_plan(&i, &RenderSettings::default(), &never);
        assert!(video_filter(&i, &RenderSettings::default(), &p).contains("in_color_matrix=bt709"));
        let sd = info(720, 576);
        let p = build_plan(&sd, &RenderSettings::default(), &never);
        assert!(video_filter(&sd, &RenderSettings::default(), &p).contains("in_color_matrix=bt601"));
    }

    #[test]
    fn alpha_warnings() {
        let mut i = info(1920, 1080);
        i.has_alpha = true;
        let s = RenderSettings { preset: "hap".into(), ..Default::default() };
        let p = build_plan(&i, &s, &never);
        assert!(p.warnings.iter().any(|w| w.key == "warn.alpha_lost"));
    }
}
