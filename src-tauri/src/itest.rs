//! Testes de integração com o ffmpeg real: geram vídeos de teste, fazem o render
//! completo (plano → ffmpeg → verificação → nome final) e confirmam o resultado.
//!
//! Precisam do ffmpeg/ffprobe no PATH:  cargo test -- --ignored

use crate::presets::{Codec, Player, RenderSettings, SizeMode};
use crate::render::{probe, render_with, Emit};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("renderpro-itest").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn gen(out: &Path, args: &[&str]) {
    let mut a = vec!["-hide_banner", "-v", "error", "-y"];
    a.extend_from_slice(args);
    let o = out.to_string_lossy().to_string();
    a.push(&o);
    let st = Command::new("ffmpeg").args(&a).status().expect("ffmpeg no PATH");
    assert!(st.success(), "gerar {out:?} falhou");
}

fn quiet() -> Emit {
    Arc::new(|_, _, _, _, _, _| {})
}

async fn run(path: &Path, s: RenderSettings) -> Result<crate::render::JobResult, String> {
    render_with(quiet(), format!("t{}", rand_id()), path.to_string_lossy().into_owned(), s).await
}

fn rand_id() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

fn leftovers(d: &Path) -> Vec<String> {
    std::fs::read_dir(d)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".renderpro-part"))
        .collect()
}

#[tokio::test]
#[ignore]
async fn playerslice_hap_pads_and_converts_audio() {
    let d = dir("ps_hap");
    let src = d.join("ledwall.mkv");
    gen(&src, &[
        "-f", "lavfi", "-i", "testsrc2=size=1918x1080:rate=50", "-f", "lavfi", "-i",
        "sine=sample_rate=44100,pan=5.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0", "-t", "2", "-c:v", "libx264",
        "-pix_fmt", "yuv420p10le", "-c:a", "aac",
    ]);
    let r = run(&src, RenderSettings::default()).await.unwrap();
    assert!(r.output.ends_with("ledwall_HAP.mov"), "{}", r.output);
    let o = probe(&r.output).await.unwrap();
    assert_eq!((o.codec.as_str(), o.codec_tag.as_str()), ("hap", "Hap1"));
    assert_eq!((o.width, o.height), (1920, 1080));
    assert_eq!((o.fps_num, o.fps_den), (50, 1));
    assert_eq!((o.audio_codec.as_str(), o.audio_channels, o.audio_rate), ("pcm_s16le", 2, 48000));
    assert!((o.duration - 2.0).abs() < 0.1, "{}", o.duration);
    assert!(leftovers(&d).is_empty());
}

#[tokio::test]
#[ignore]
async fn alpha_goes_to_hap_alpha() {
    let d = dir("alpha");
    let src = d.join("logo.mov");
    gen(&src, &["-f", "lavfi", "-i", "testsrc2=size=721x647:rate=25,format=argb", "-t", "1", "-c:v", "qtrle"]);
    let r = run(&src, RenderSettings::default()).await.unwrap();
    let o = probe(&r.output).await.unwrap();
    assert_eq!(o.codec_tag, "Hap5");
    assert_eq!((o.width, o.height), (724, 648));
    assert!(r.warnings.iter().any(|w| w.key == "warn.pad"));
}

#[tokio::test]
#[ignore]
async fn videohost_h264_from_vfr_and_interlaced() {
    let d = dir("vh");
    let vfr = d.join("phone.mkv");
    gen(&vfr, &[
        "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-f", "lavfi", "-i", "sine", "-t", "3", "-vf",
        "setpts='(N+0.45*mod(N\\,2))/(30*TB)'", "-fps_mode", "passthrough", "-c:v", "libx264", "-c:a", "aac",
    ]);
    let s = RenderSettings { player: Player::Videohost, ..Default::default() };
    let r = run(&vfr, s.clone()).await.unwrap();
    let o = probe(&r.output).await.unwrap();
    assert_eq!(o.codec, "h264");
    assert_eq!(o.pix_fmt, "yuv420p");
    assert_eq!(o.profile, "High");
    assert!(!o.vfr);
    assert_eq!(o.audio_codec, "aac");

    let int = d.join("broadcast.mov");
    gen(&int, &[
        "-f", "lavfi", "-i", "testsrc2=size=720x576:rate=25", "-t", "2", "-vf", "setfield=tff", "-c:v", "libx264",
        "-flags", "+ildct+ilme", "-x264-params", "tff=1", "-pix_fmt", "yuv420p",
    ]);
    let info = probe(&int.to_string_lossy()).await.unwrap();
    assert!(info.interlaced, "o ficheiro de teste devia ser entrelaçado");
    let r = run(&int, s).await.unwrap();
    assert!(r.warnings.iter().any(|w| w.key == "warn.deinterlaced"));
    let o = probe(&r.output).await.unwrap();
    assert!(!o.interlaced);
}

#[tokio::test]
#[ignore]
async fn videohost_4k_goes_to_hap_with_chunks() {
    let d = dir("4k");
    let src = d.join("big.mp4");
    gen(&src, &["-f", "lavfi", "-i", "testsrc2=size=3840x2160:rate=25", "-t", "1", "-c:v", "libx264", "-preset", "ultrafast"]);
    let s = RenderSettings { player: Player::Videohost, ..Default::default() };
    let r = run(&src, s).await.unwrap();
    assert!(r.output.ends_with("big_HAP.mov"));
}

#[tokio::test]
#[ignore]
async fn every_codec_renders_and_verifies() {
    let d = dir("all");
    let src = d.join("src.mov");
    gen(&src, &[
        "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25,format=yuva444p", "-f", "lavfi", "-i", "sine", "-t", "1",
        "-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", "yuva444p10le", "-c:a", "pcm_s24le",
    ]);
    for c in crate::presets::ALL_CODECS {
        let s = RenderSettings { player: Player::Custom, preset: c.id().into(), ..Default::default() };
        let r = run(&src, s).await.unwrap_or_else(|e| panic!("{}: {e}", c.label()));
        let o = probe(&r.output).await.unwrap();
        assert_eq!(o.codec, c.probe_name(), "{}", c.label());
        if *c == Codec::Prores4444 {
            assert!(o.has_alpha, "ProRes 4444 devia manter o alfa");
        }
        assert!(o.audio_index.is_some(), "{} sem som", c.label());
    }
    assert!(leftovers(&d).is_empty());
}

#[tokio::test]
#[ignore]
async fn loudness_normalization_hits_target() {
    let d = dir("loud");
    let src = d.join("quiet.mp4");
    gen(&src, &[
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25", "-f", "lavfi", "-i", "sine=frequency=500,volume=0.05",
        "-t", "8", "-c:v", "libx264", "-c:a", "aac",
    ]);
    let s = RenderSettings { player: Player::Videohost, normalize: true, lufs: -16.0, ..Default::default() };
    let r = run(&src, s).await.unwrap();
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i", &r.output, "-af", "ebur128", "-f", "null", "-"])
        .output()
        .unwrap();
    let txt = String::from_utf8_lossy(&out.stderr);
    let i_line = txt.lines().filter(|l| l.trim_start().starts_with("I:")).last().unwrap();
    let lufs: f64 = i_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!((lufs + 16.0).abs() < 1.5, "loudness {lufs}");
}

#[tokio::test]
#[ignore]
async fn names_never_clash_and_custom_size() {
    let d = dir("names");
    let src = d.join("a.mp4");
    gen(&src, &["-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=25", "-t", "1", "-c:v", "libx264"]);
    let s = RenderSettings {
        size: SizeMode::Custom,
        width: 4864,
        height: 936,
        fit: crate::presets::Fit::Fit,
        verify: false,
        ..Default::default()
    };
    let r1 = run(&src, s.clone()).await.unwrap();
    let r2 = run(&src, s).await.unwrap();
    assert_ne!(r1.output, r2.output);
    assert!(r2.output.ends_with("a_HAP (2).mov"), "{}", r2.output);
    let o = probe(&r2.output).await.unwrap();
    assert_eq!((o.width, o.height), (4864, 936));
}

#[tokio::test]
#[ignore]
async fn cancel_leaves_nothing_behind() {
    let d = dir("cancel");
    let src = d.join("long.mp4");
    gen(&src, &["-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=25", "-t", "40", "-c:v", "libx264", "-preset", "ultrafast"]);
    let id = "cancel-me".to_string();
    let p = src.to_string_lossy().into_owned();
    let s = RenderSettings { preset: "prores_hq".into(), ..Default::default() };
    let h = tokio::spawn(render_with(quiet(), id.clone(), p, s));
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(crate::render::cancel(&id));
    let r = h.await.unwrap();
    assert_eq!(r.unwrap_err(), "cancelled");
    let names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names, vec!["long.mp4".to_string()], "sobrou: {names:?}");
}

#[tokio::test]
#[ignore]
async fn broken_input_never_produces_output() {
    let d = dir("broken");
    let src = d.join("good.mov");
    gen(&src, &["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25", "-t", "4", "-c:v", "prores_ks"]);
    // corta o ficheiro a meio (cópia interrompida)
    let bytes = std::fs::read(&src).unwrap();
    let cut = d.join("cut.mov");
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    let r = run(&cut, RenderSettings::default()).await;
    match r {
        Err(_) => {}
        // se o ffmpeg conseguir ler o princípio, o ficheiro final tem de estar completo e verificado
        Ok(res) => {
            let o = probe(&res.output).await.unwrap();
            assert_eq!(o.codec, "hap");
        }
    }
    assert!(leftovers(&d).is_empty());
    std::fs::write(d.join("lixo.mov"), b"isto nao e um video").unwrap();
    assert!(run(&d.join("lixo.mov"), RenderSettings::default()).await.is_err());
    assert!(!d.join("lixo_HAP.mov").exists());
}
