//! Conversor de video: porta em Rust do conversor.pyw, usando o ffmpeg do PATH.
//! Nunca sobrescreve e nunca toca no original: a saida sempre tem um nome livre.

use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const NO_WINDOW: u32 = 0x0800_0000;
pub const FFMPEG_MISSING: &str = "err.ffmpegMissing";
const VIDEO_CONTAINERS: [&str; 3] = ["mp4", "mov", "mkv"];
const OUTPUT_FORMATS: [&str; 10] = ["mp4", "mov", "mkv", "webm", "gif", "mp3", "wav", "m4a", "flac", "opus"];
const MEDIA_EXTENSIONS: [&str; 20] = [
    "opus",
    "mkv", "mp4", "mov", "avi", "webm", "flv", "wmv", "m4v", "ts", "mts", "m2ts", "3gp", "mp3", "wav", "flac", "m4a", "ogg", "aac", "mpg",
];

#[derive(Debug, PartialEq)]
pub enum Error {
    Cancelled,
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Copy {
    All,
    Video,
    None,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub ext: String,
    pub mode: String, // auto | copy | encode
    pub crf: u32,
    pub gpu: Option<String>,
    pub out_dir: Option<PathBuf>,
}

pub fn is_output_format(ext: &str) -> bool {
    OUTPUT_FORMATS.contains(&ext)
}

pub fn is_media(path: &Path) -> bool {
    extension(path).is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.as_str()))
}

pub fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Procura no PATH do Windows.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|dir| dir.join(format!("{name}.exe"))).find(|p| p.is_file())
}

/// Nome livre para a saida; o original e qualquer arquivo existente ficam intactos.
pub fn output_path(src: &Path, ext: &str, out_dir: Option<&Path>) -> PathBuf {
    let folder = out_dir.map(Path::to_path_buf).or_else(|| src.parent().map(Path::to_path_buf)).unwrap_or_default();
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "saida".into());
    let mut candidate = folder.join(format!("{stem}.{ext}"));
    let mut n = 1;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    candidate
}

pub fn build_args(ext: &str, copy: Copy, crf: u32, gpu: Option<&str>) -> Vec<String> {
    let strs = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match ext {
        "mp3" => return strs(&["-vn", "-c:a", "libmp3lame", "-q:a", "2"]),
        "wav" => return strs(&["-vn", "-c:a", "pcm_s16le"]),
        "flac" => return strs(&["-vn", "-c:a", "flac"]),
        "opus" => return strs(&["-vn", "-c:a", "libopus", "-b:a", "192k"]),
        "m4a" => return strs(&["-vn", "-c:a", "aac", "-b:a", "192k"]),
        "gif" => return strs(&["-vf", "fps=15,scale=480:-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse", "-loop", "0"]),
        "webm" => {
            let crf = (crf + 10).to_string();
            return strs(&["-map", "0:v:0", "-map", "0:a?", "-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0", "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k"]);
        }
        _ => {}
    }
    let mut args = strs(&["-map", "0:v:0", "-map", "0:a?"]);
    let crf_text = crf.to_string();
    match copy {
        Copy::All => args.extend(strs(&["-c", "copy"])),
        Copy::Video => args.extend(strs(&["-c:v", "copy", "-c:a", "aac", "-b:a", "192k"])),
        Copy::None => {
            let encoder: Vec<&str> = match gpu {
                Some("h264_nvenc") => vec!["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", &crf_text, "-b:v", "0"],
                Some("h264_amf") => vec!["-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", &crf_text, "-qp_p", &crf_text],
                Some("h264_qsv") => vec!["-c:v", "h264_qsv", "-global_quality", &crf_text],
                _ => vec!["-c:v", "libx264", "-preset", "medium", "-crf", &crf_text],
            };
            args.extend(strs(&encoder));
            args.extend(strs(&["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"]));
        }
    }
    if ext == "mp4" || ext == "mov" {
        args.extend(strs(&["-movflags", "+faststart"]));
    }
    args
}

/// Ordem das tentativas: copiar tudo, copiar so o video e, por fim, reencodar.
pub fn attempts(ext: &str, mode: &str) -> Vec<Copy> {
    let container = VIDEO_CONTAINERS.contains(&ext);
    let mut list = Vec::new();
    if container && (mode == "auto" || mode == "copy") {
        list.extend([Copy::All, Copy::Video]);
    }
    if !container || mode == "auto" || mode == "encode" {
        list.push(Copy::None);
    }
    list
}

fn quiet_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(NO_WINDOW).stdin(Stdio::null());
    command
}

/// Codec do primeiro video do arquivo; None se nao houver video ou o ffprobe nao estiver disponivel.
pub fn video_codec(path: &Path) -> Option<String> {
    let ffprobe = find_tool("ffprobe")?;
    let output = quiet_command(&ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let codec = String::from_utf8_lossy(&output.stdout).trim().trim_end_matches(',').to_string();
    (!codec.is_empty()).then_some(codec)
}

/// MP4 e MOV sao pedidos para edicao: video em outro codec (VP9, AV1) precisa virar H.264.
pub fn needs_h264(path: &Path, ext: &str) -> bool {
    matches!(ext, "mp4" | "mov") && video_codec(path).is_some_and(|codec| codec != "h264")
}

/// Primeiro encoder H.264 de GPU que realmente funciona nesta maquina.
pub fn detect_gpu(ffmpeg: &Path) -> Option<String> {
    ["h264_nvenc", "h264_amf", "h264_qsv"].iter().find_map(|encoder| {
        let status = quiet_command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=s=256x256:d=0.2", "-c:v", encoder, "-f", "null", "-"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        status.ok().filter(|s| s.success()).map(|_| encoder.to_string())
    })
}

fn duration_of(src: &Path) -> Option<f64> {
    let ffprobe = find_tool("ffprobe")?;
    let output = quiet_command(&ffprobe)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(src)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn run_ffmpeg(
    ffmpeg: &Path,
    src: &Path,
    dst: &Path,
    args: &[String],
    duration: Option<f64>,
    cancel: &AtomicBool,
    on_percent: &mut dyn FnMut(f64),
) -> Result<(), Error> {
    let mut child = quiet_command(ffmpeg)
        .args(["-hide_banner", "-n", "-i"])
        .arg(src)
        .args(args)
        .args(["-progress", "pipe:1", "-nostats"])
        .arg(dst)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Message(crate::i18n::code("err.ffmpegStart", &[&e.to_string()])))?;

    let stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let errors = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    });
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                if let Ok(micro) = value.trim().parse::<f64>() {
                    let _ = sender.send(micro / 1_000_000.0);
                }
            }
        }
    });

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = errors.join();
            return Err(Error::Cancelled);
        }
        if let Ok(seconds) = receiver.recv_timeout(Duration::from_millis(100)) {
            if let Some(total) = duration.filter(|d| *d > 0.0) {
                on_percent((seconds / total * 100.0).min(100.0));
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            let text = errors.join().unwrap_or_default();
            if status.success() {
                return Ok(());
            }
            let tail: Vec<&str> = text.lines().rev().take(2).collect();
            let message = tail.into_iter().rev().collect::<Vec<_>>().join(" ");
            return Err(Error::Message(if message.is_empty() { "err.ffmpegFailed".into() } else { message }));
        }
    }
}

/// Converte um arquivo e devolve o caminho da saida. Em falha, so remove a propria saida parcial.
pub fn convert_file(
    src: &Path,
    options: &Options,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(f64, &str),
) -> Result<PathBuf, Error> {
    let ffmpeg = find_tool("ffmpeg").ok_or_else(|| Error::Message(FFMPEG_MISSING.into()))?;
    if !src.is_file() {
        return Err(Error::Message("err.fileNotFound".into()));
    }
    if let Some(dir) = &options.out_dir {
        std::fs::create_dir_all(dir).map_err(|e| Error::Message(crate::i18n::code("err.outDir", &[&e.to_string()])))?;
    }
    let dst = output_path(src, &options.ext, options.out_dir.as_deref());
    let duration = duration_of(src);
    let mut last_error = Error::Message("err.noMode".into());

    for copy in attempts(&options.ext, &options.mode) {
        let gpus: Vec<Option<&str>> = match (copy, options.gpu.as_deref()) {
            (Copy::None, Some(gpu)) => vec![Some(gpu), None],
            _ => vec![None],
        };
        for gpu in gpus {
            let how = match (copy, gpu) {
                (Copy::All, _) => "conv.copy".to_string(),
                (Copy::Video, _) => "conv.copyVideo".to_string(),
                (Copy::None, Some(g)) => crate::i18n::code("conv.encodeGpu", &[g]),
                (Copy::None, None) => "conv.encodeCpu".to_string(),
            };
            on_progress(0.0, &how);
            let args = build_args(&options.ext, copy, options.crf, gpu);
            let result = run_ffmpeg(&ffmpeg, src, &dst, &args, duration, cancel, &mut |percent| on_progress(percent, &how));
            match result {
                Ok(()) => return Ok(dst),
                Err(error) => {
                    let _ = std::fs::remove_file(&dst);
                    if error == Error::Cancelled {
                        return Err(Error::Cancelled);
                    }
                    last_error = error;
                }
            }
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("frey-convert-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn options(ext: &str, mode: &str) -> Options {
        Options { ext: ext.into(), mode: mode.into(), crf: 22, gpu: None, out_dir: None }
    }

    /// Video curto gerado pelo proprio ffmpeg; sem ffmpeg o teste nao tem o que provar.
    fn sample(dir: &Path, seconds: &str) -> Option<PathBuf> {
        let ffmpeg = find_tool("ffmpeg")?;
        let path = dir.join("origem.mp4");
        let status = quiet_command(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc=d={seconds}:s=320x240:r=30"), "-f", "lavfi", "-i", &format!("sine=d={seconds}"), "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
            .arg(&path)
            .status()
            .ok()?;
        status.success().then_some(path)
    }

    #[test]
    fn saida_nunca_usa_nome_existente() {
        let dir = temp_dir("names");
        let src = dir.join("clipe.mov");
        std::fs::write(&src, "x").unwrap();
        assert_eq!(output_path(&src, "mov", None), dir.join("clipe (1).mov"));
        assert_eq!(output_path(&src, "mp4", None), dir.join("clipe.mp4"));
        std::fs::write(dir.join("clipe.mp4"), "x").unwrap();
        std::fs::write(dir.join("clipe (1).mp4"), "x").unwrap();
        assert_eq!(output_path(&src, "mp4", None), dir.join("clipe (2).mp4"));
        assert_eq!(output_path(&src, "mp4", Some(Path::new("Z:\\out"))), Path::new("Z:\\out\\clipe.mp4"));
    }

    #[test]
    fn tentativas_seguem_o_modo() {
        assert_eq!(attempts("mp4", "auto"), vec![Copy::All, Copy::Video, Copy::None]);
        assert_eq!(attempts("mov", "copy"), vec![Copy::All, Copy::Video]);
        assert_eq!(attempts("mkv", "encode"), vec![Copy::None]);
        assert_eq!(attempts("mp3", "copy"), vec![Copy::None]);
        assert_eq!(attempts("webm", "auto"), vec![Copy::None]);
    }

    #[test]
    fn argumentos_iguais_aos_do_conversor_original() {
        assert_eq!(build_args("mp3", Copy::None, 22, None), ["-vn", "-c:a", "libmp3lame", "-q:a", "2"]);
        assert_eq!(build_args("mkv", Copy::All, 22, None), ["-map", "0:v:0", "-map", "0:a?", "-c", "copy"]);
        let mp4 = build_args("mp4", Copy::None, 18, None);
        assert!(mp4.windows(2).any(|w| w == ["-crf", "18"]));
        assert_eq!(mp4[mp4.len() - 2..], ["-movflags", "+faststart"]);
        assert!(build_args("mp4", Copy::None, 22, Some("h264_nvenc")).windows(2).any(|w| w == ["-cq", "22"]));
        assert!(build_args("webm", Copy::None, 22, None).windows(2).any(|w| w == ["-crf", "32"]));
    }

    #[test]
    fn reconhece_midia_e_formatos() {
        assert!(is_media(Path::new("a.MOV")));
        assert!(!is_media(Path::new("briefing.txt")));
        assert!(is_output_format("gif"));
        assert!(!is_output_format("exe"));
    }

    #[test]
    fn converte_de_verdade_e_mantem_o_original() {
        let dir = temp_dir("real");
        let Some(src) = sample(&dir, "1") else { return };
        let before = std::fs::read(&src).unwrap();
        for (ext, mode) in [("mov", "copy"), ("mkv", "auto"), ("mp3", "auto"), ("wav", "auto"), ("flac", "auto"), ("opus", "auto"), ("gif", "auto")] {
            let mut percents = Vec::new();
            let out = convert_file(&src, &options(ext, mode), &AtomicBool::new(false), &mut |p, _| percents.push(p)).unwrap();
            assert_eq!(extension(&out).unwrap(), ext);
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{ext}");
            assert!(!percents.is_empty());
        }
        assert_eq!(std::fs::read(&src).unwrap(), before);
        // Mesmo formato: nao sobrescreve, cria com outro nome.
        let again = convert_file(&src, &options("mp4", "encode"), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(again, dir.join("origem (1).mp4"));
        assert_eq!(std::fs::read(&src).unwrap(), before);
    }

    #[test]
    fn reconhece_codec_e_pede_h264_so_para_edicao() {
        let dir = temp_dir("codec");
        let Some(h264) = sample(&dir, "1") else { return };
        assert_eq!(video_codec(&h264).as_deref(), Some("h264"));
        assert!(!needs_h264(&h264, "mp4"));
        let vp9 = convert_file(&h264, &options("webm", "encode"), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(video_codec(&vp9).as_deref(), Some("vp9"));
        assert!(needs_h264(&vp9, "mp4"));
        assert!(needs_h264(&vp9, "mov"));
        assert!(!needs_h264(&vp9, "mkv"));
        let audio = convert_file(&h264, &options("wav", "auto"), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(video_codec(&audio), None);
        assert!(!needs_h264(&audio, "mp4"));
    }

    #[test]
    fn arquivo_invalido_falha_e_nao_deixa_lixo() {
        let dir = temp_dir("bad");
        if find_tool("ffmpeg").is_none() {
            return;
        }
        let src = dir.join("falso.mp4");
        std::fs::write(&src, "nao sou video").unwrap();
        let result = convert_file(&src, &options("mov", "auto"), &AtomicBool::new(false), &mut |_, _| {});
        assert!(matches!(result, Err(Error::Message(_))));
        assert!(!dir.join("falso.mov").exists());
    }

    #[test]
    fn cancelar_encerra_o_ffmpeg_e_remove_a_saida() {
        let dir = temp_dir("cancel");
        let Some(src) = sample(&dir, "30") else { return };
        let cancel = AtomicBool::new(false);
        let result = convert_file(&src, &options("webm", "encode"), &cancel, &mut |_, _| cancel.store(true, Ordering::Relaxed));
        assert_eq!(result, Err(Error::Cancelled));
        assert!(!dir.join("origem.webm").exists());
        assert!(src.exists());
    }
}
