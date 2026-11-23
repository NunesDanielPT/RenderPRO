//! Leitura do que está dentro de um ficheiro de vídeo (saída JSON do ffprobe).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Extensões aceites ao arrastar pastas.
pub const VIDEO_EXTS: &[&str] = &[
    "mov", "mp4", "m4v", "mkv", "avi", "mxf", "webm", "wmv", "mpg", "mpeg", "m2ts", "mts", "ts", "flv", "3gp",
    "ogv", "dv", "vob", "gif",
];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub path: String,
    pub size: u64,
    pub container: String,
    /// Duração em segundos (0 se desconhecida).
    pub duration: f64,
    pub video_index: Option<u32>,
    pub codec: String,
    /// Nome legível (ex. "HAP Q", "ProRes 422 HQ", "H.264").
    pub codec_label: String,
    pub codec_tag: String,
    pub profile: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub bit_depth: u32,
    pub has_alpha: bool,
    pub fps_num: u32,
    pub fps_den: u32,
    /// avg_frame_rate muito diferente de r_frame_rate → frame rate variável (telemóveis, gravações de ecrã).
    pub vfr: bool,
    pub interlaced: bool,
    pub rotation: i32,
    pub sar_num: u32,
    pub sar_den: u32,
    pub color_space: String,
    pub color_transfer: String,
    pub color_range: String,
    pub hdr: bool,
    pub frames: u64,
    pub bitrate: u64,
    pub audio_index: Option<u32>,
    pub audio_codec: String,
    pub audio_channels: u32,
    pub audio_rate: u32,
    pub audio_streams: u32,
}

impl MediaInfo {
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 {
            0.0
        } else {
            self.fps_num as f64 / self.fps_den as f64
        }
    }
    /// Dimensões como aparecem no ecrã (rotação e píxeis não quadrados aplicados).
    pub fn display_size(&self) -> (u32, u32) {
        let (mut w, mut h) = (self.width, self.height);
        if self.sar_num > 0 && self.sar_den > 0 && self.sar_num != self.sar_den {
            w = ((w as f64) * self.sar_num as f64 / self.sar_den as f64).round().max(1.0) as u32;
        }
        if self.rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut w, &mut h);
        }
        (w, h)
    }
    pub fn has_audio(&self) -> bool {
        self.audio_index.is_some()
    }
}

/// "30000/1001" → (30000, 1001).
pub fn parse_ratio(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(['/', ':'])?;
    let a: u64 = a.trim().parse().ok()?;
    let b: u64 = b.trim().parse().ok()?;
    if a == 0 || b == 0 || a > u32::MAX as u64 || b > u32::MAX as u64 {
        return None;
    }
    Some((a as u32, b as u32))
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// Formatos de píxel com canal alfa.
pub fn pix_fmt_has_alpha(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.starts_with("yuva")
        || p.starts_with("gbrap")
        || p.starts_with("ya")
        || p.contains("rgba")
        || p.contains("bgra")
        || p.contains("argb")
        || p.contains("abgr")
        || p.starts_with("ayuv")
        || p == "pal8"
}

fn bit_depth(p: &str) -> u32 {
    for d in ["16", "14", "12", "10", "9"] {
        if p.contains(&format!("p{d}")) || p.ends_with(&format!("{d}le")) || p.ends_with(&format!("{d}be")) {
            return d.parse().unwrap_or(8);
        }
    }
    8
}

pub fn codec_label(codec: &str, tag: &str, profile: &str) -> String {
    match codec {
        "hap" => match tag {
            "Hap5" => "HAP Alpha",
            "HapY" => "HAP Q",
            "HapM" => "HAP Q Alpha",
            "Hap7" => "HAP R",
            "HapA" => "HAP Alpha-only",
            _ => "HAP",
        }
        .into(),
        "h264" => "H.264".into(),
        "hevc" => "H.265".into(),
        "prores" => {
            if profile.is_empty() {
                "ProRes".into()
            } else {
                format!("ProRes {}", profile.replace("4444XQ", "4444 XQ"))
            }
        }
        "dnxhd" => {
            let p = profile.to_ascii_uppercase();
            if p.starts_with("DNXHR") {
                format!("DNxHR {}", p.trim_start_matches("DNXHR").trim())
            } else {
                "DNxHD".into()
            }
        }
        "vp9" => "VP9".into(),
        "vp8" => "VP8".into(),
        "av1" => "AV1".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg4" => "MPEG-4".into(),
        "qtrle" => "QuickTime Animation".into(),
        "png" => "PNG".into(),
        "mjpeg" => "Motion JPEG".into(),
        "cfhd" => "CineForm".into(),
        "dxv" => "DXV".into(),
        "rawvideo" => "Sem compressão".into(),
        "" => "—".into(),
        other => other.to_uppercase(),
    }
}

/// Interpreta `ffprobe -print_format json -show_format -show_streams`.
pub fn parse_probe(path: &str, file_size: u64, v: &Value) -> Result<MediaInfo, String> {
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let fmt = &v["format"];
    let mut m = MediaInfo {
        path: path.to_string(),
        size: file_size,
        container: s(&fmt["format_name"]),
        duration: num(&fmt["duration"]).unwrap_or(0.0),
        bitrate: num(&fmt["bit_rate"]).unwrap_or(0.0) as u64,
        sar_num: 1,
        sar_den: 1,
        ..Default::default()
    };

    // Primeiro vídeo que não seja uma capa (attached_pic).
    let video = streams.iter().find(|st| {
        st["codec_type"] == "video" && st["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    });
    let Some(vs) = video else {
        return Err("novideo".into());
    };
    m.video_index = vs["index"].as_u64().map(|i| i as u32);
    m.codec = s(&vs["codec_name"]);
    m.codec_tag = s(&vs["codec_tag_string"]);
    m.profile = s(&vs["profile"]);
    m.codec_label = codec_label(&m.codec, &m.codec_tag, &m.profile);
    m.width = vs["width"].as_u64().unwrap_or(0) as u32;
    m.height = vs["height"].as_u64().unwrap_or(0) as u32;
    if m.width == 0 || m.height == 0 {
        return Err("novideo".into());
    }
    m.pix_fmt = s(&vs["pix_fmt"]);
    m.bit_depth = vs["bits_per_raw_sample"]
        .as_str()
        .and_then(|b| b.parse().ok())
        .unwrap_or_else(|| bit_depth(&m.pix_fmt));
    m.has_alpha = pix_fmt_has_alpha(&m.pix_fmt) || matches!(m.codec_tag.as_str(), "Hap5" | "HapM" | "HapA");
    // HAP: o ffprobe diz rgb0/rgba conforme o tipo — o tag manda.
    if m.codec == "hap" {
        m.has_alpha = matches!(m.codec_tag.as_str(), "Hap5" | "HapM" | "HapA");
    }

    let avg = parse_ratio(&s(&vs["avg_frame_rate"]));
    let real = parse_ratio(&s(&vs["r_frame_rate"]));
    let chosen = avg.or(real).unwrap_or((25, 1));
    let chosen = if (chosen.0 as f64 / chosen.1 as f64) > 1000.0 { real.unwrap_or((25, 1)) } else { chosen };
    m.fps_num = chosen.0;
    m.fps_den = chosen.1;
    if let (Some(a), Some(r)) = (avg, real) {
        let fa = a.0 as f64 / a.1 as f64;
        let fr = r.0 as f64 / r.1 as f64;
        m.vfr = fr > 0.0 && (fa - fr).abs() / fr > 0.02;
    }

    let fo = s(&vs["field_order"]);
    m.interlaced = matches!(fo.as_str(), "tt" | "bb" | "tb" | "bt");

    // Rotação: side_data (ffmpeg recente) ou tag "rotate" (antigo).
    let mut rot = vs["tags"]["rotate"].as_str().and_then(|r| r.parse::<i32>().ok()).unwrap_or(0);
    if let Some(sd) = vs["side_data_list"].as_array() {
        for d in sd {
            if let Some(r) = num(&d["rotation"]) {
                rot = r.round() as i32;
            }
        }
    }
    m.rotation = rot.rem_euclid(360);

    if let Some((a, b)) = parse_ratio(&s(&vs["sample_aspect_ratio"])) {
        m.sar_num = a;
        m.sar_den = b;
    }
    m.color_space = s(&vs["color_space"]);
    m.color_transfer = s(&vs["color_transfer"]);
    m.color_range = s(&vs["color_range"]);
    m.hdr = matches!(m.color_transfer.as_str(), "smpte2084" | "arib-std-b67");

    if let Some(d) = num(&vs["duration"]) {
        if m.duration <= 0.0 {
            m.duration = d;
        }
    }
    m.frames = vs["nb_frames"].as_str().and_then(|f| f.parse().ok()).unwrap_or(0);
    if m.frames == 0 && m.duration > 0.0 {
        m.frames = (m.duration * m.fps()).round() as u64;
    }

    let audios: Vec<&Value> = streams.iter().filter(|st| st["codec_type"] == "audio").collect();
    m.audio_streams = audios.len() as u32;
    if let Some(a) = audios.first() {
        m.audio_index = a["index"].as_u64().map(|i| i as u32);
        m.audio_codec = s(&a["codec_name"]);
        m.audio_channels = a["channels"].as_u64().unwrap_or(0) as u32;
        m.audio_rate = num(&a["sample_rate"]).unwrap_or(0.0) as u32;
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ratios() {
        assert_eq!(parse_ratio("30000/1001"), Some((30000, 1001)));
        assert_eq!(parse_ratio("16:9"), Some((16, 9)));
        assert_eq!(parse_ratio("0/0"), None);
        assert_eq!(parse_ratio("x"), None);
    }

    #[test]
    fn alpha_formats() {
        for p in ["yuva444p10le", "rgba", "argb", "bgra", "gbrap", "ya8"] {
            assert!(pix_fmt_has_alpha(p), "{p}");
        }
        for p in ["yuv420p", "yuv422p10le", "rgb24", "rgb0", "gbrp"] {
            assert!(!pix_fmt_has_alpha(p), "{p}");
        }
    }

    #[test]
    fn parses_typical_file() {
        let v = json!({
            "streams": [
                {"index":0,"codec_type":"video","codec_name":"mjpeg","width":300,"height":300,"disposition":{"attached_pic":1}},
                {"index":1,"codec_type":"video","codec_name":"h264","codec_tag_string":"avc1","profile":"High",
                 "width":1920,"height":1080,"pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001",
                 "field_order":"progressive","sample_aspect_ratio":"1:1","color_space":"bt709","nb_frames":"300",
                 "side_data_list":[{"rotation":-90}],"disposition":{"attached_pic":0}},
                {"index":2,"codec_type":"audio","codec_name":"aac","channels":6,"sample_rate":"48000"}
            ],
            "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10.01","bit_rate":"8000000"}
        });
        let m = parse_probe("a.mp4", 10, &v).unwrap();
        assert_eq!(m.video_index, Some(1));
        assert_eq!(m.codec_label, "H.264");
        assert_eq!((m.fps_num, m.fps_den), (30000, 1001));
        assert_eq!(m.rotation, 270);
        assert_eq!(m.display_size(), (1080, 1920));
        assert_eq!(m.audio_index, Some(2));
        assert_eq!(m.audio_channels, 6);
        assert!(!m.has_alpha && !m.vfr && !m.interlaced);
        assert_eq!(m.frames, 300);
    }

    #[test]
    fn detects_vfr_interlace_hap_and_anamorphic() {
        let v = json!({
            "streams": [{"index":0,"codec_type":"video","codec_name":"hap","codec_tag_string":"Hap5",
                "width":720,"height":576,"pix_fmt":"rgba","avg_frame_rate":"2997/100","r_frame_rate":"60/1",
                "field_order":"tt","sample_aspect_ratio":"16:15"}],
            "format": {"duration":"2"}
        });
        let m = parse_probe("a.mov", 1, &v).unwrap();
        assert!(m.vfr && m.interlaced && m.has_alpha);
        assert_eq!(m.codec_label, "HAP Alpha");
        assert_eq!(m.display_size(), (768, 576));
    }

    #[test]
    fn no_video_is_error() {
        let v = json!({"streams":[{"index":0,"codec_type":"audio","codec_name":"mp3"}],"format":{}});
        assert_eq!(parse_probe("a.mp3", 1, &v).unwrap_err(), "novideo");
    }

    #[test]
    fn labels() {
        assert_eq!(codec_label("prores", "apch", "HQ"), "ProRes HQ");
        assert_eq!(codec_label("dnxhd", "AVdh", "DNXHR HQX"), "DNxHR HQX");
        assert_eq!(codec_label("hap", "HapY", ""), "HAP Q");
    }
}
