//! Download de links do YouTube pelo yt-dlp instalado no PATH.
//! O YouTube muda com frequencia; quem acompanha isso e o yt-dlp, nao este app.
//! Nunca sobrescreve (--no-overwrites), retoma parciais e nao usa cookies do usuario.

use crate::convert::find_tool;
use crate::drive::{Error, Outcome, Progress};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const NO_WINDOW: u32 = 0x0800_0000;
pub const MISSING: &str = "err.ytMissing";
const PROGRESS_PREFIX: &str = "FREYPROG|";
const DONE_PREFIX: &str = "FREYDONE|";

/// Argumentos da qualidade escolhida. H.264/AAC por padrao: e o que entra direto na edicao.
pub fn quality_args(quality: &str) -> Vec<&'static str> {
    match quality {
        "720" => vec!["-S", "vcodec:h264,res:720,acodec:aac", "--merge-output-format", "mp4"],
        "best" => vec!["-S", "res,fps"],
        // Maior resolucao ate 4K, em SDR: o app reencoda depois para H.264 de 8 bits, e HDR ficaria com a cor errada.
        "edit4k" => vec!["-S", "hdr:sdr,res:2160,fps", "--merge-output-format", "mkv"],
        "audio" => vec!["-f", "ba/b", "-x", "--audio-format", "m4a"],
        _ => vec!["-S", "vcodec:h264,res:1080,acodec:aac", "--merge-output-format", "mp4"],
    }
}

fn build_args(url: &str, dest: &Path, quality: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["-P".into(), dest.display().to_string()];
    let fixed = [
        "--ignore-config", "--no-playlist", "--no-overwrites", "--continue", "--newline", "--encoding", "utf-8",
        "-o", "%(title)s.%(ext)s",
        "--no-simulate", "--progress",
        "--progress-template",
        "download:FREYPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.title)s",
        "--print", "after_move:FREYDONE|%(filepath)s",
    ];
    args.extend(fixed.iter().map(|s| s.to_string()));
    args.extend(quality_args(quality).iter().map(|s| s.to_string()));
    args.push("--".into());
    args.push(url.to_string());
    args
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

/// Le uma linha de progresso do nosso modelo; campos ausentes vem como "NA".
pub fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = rest.splitn(6, '|');
    let done = number(parts.next()?)? as u64;
    let total = number(parts.next()?);
    let estimate = number(parts.next()?);
    let index = number(parts.next()?).map_or(1, |n| n as usize).max(1);
    let entries = number(parts.next()?).map_or(1, |n| n as usize).max(1);
    let title = parts.next()?.trim().to_string();
    Some(Progress {
        file: title,
        done,
        total: total.or(estimate).map(|t| t as u64),
        files_done: index - 1,
        files_total: entries,
    })
}

/// Explica os erros comuns; o resto segue como o yt-dlp escreveu.
pub fn friendly_error(stderr: &str) -> String {
    let last = stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("err.ytFailed")
        .trim();
    let lower = last.to_lowercase();
    if lower.contains("sign in") || lower.contains("cookies") {
        "err.ytLogin".into()
    } else if lower.contains("private video") {
        "err.ytPrivate".into()
    } else if lower.contains("video unavailable") || lower.contains("not available") {
        "err.ytUnavailable".into()
    } else if lower.contains("drm") {
        "err.ytDrm".into()
    } else if lower.contains("ffmpeg") {
        "err.ffmpegMissing".into()
    } else if lower.contains("unable to download") || lower.contains("timed out") || lower.contains("http error") {
        crate::i18n::code("err.ytConn", &[last])
    } else {
        last.trim_start_matches("ERROR:").trim().chars().take(240).collect()
    }
}

fn kill_tree(pid: u32) {
    // yt-dlp abre o ffmpeg como filho: encerrar so ele deixaria o filho rodando.
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn existing_files(dest: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(dest)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default()
}

pub fn download(url: &str, dest: &Path, quality: &str, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    let tool = find_tool("yt-dlp").ok_or_else(|| Error::Message(MISSING.into()))?;
    run(&tool, url, dest, quality, cancel, on_progress)
}

fn run(tool: &Path, url: &str, dest: &Path, quality: &str, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    std::fs::create_dir_all(dest)?;
    let before = existing_files(dest);
    let mut child = Command::new(tool)
        .args(build_args(url, dest, quality))
        .env("PYTHONUTF8", "1")
        .creation_flags(NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Message(crate::i18n::code("err.ytStart", &[&e.to_string()])))?;

    let stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let errors = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    });
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).split(b'\n').map_while(Result::ok) {
            let _ = sender.send(String::from_utf8_lossy(&line).trim_end().to_string());
        }
    });

    let mut outcome = Outcome { downloaded: 0, skipped: 0, files: Vec::new() };
    let mut handle = |line: String, outcome: &mut Outcome| {
        if let Some(path) = line.strip_prefix(DONE_PREFIX) {
            let path = PathBuf::from(path.trim());
            if before.contains(&path) {
                outcome.skipped += 1;
            } else {
                outcome.downloaded += 1;
                outcome.files.push(path);
            }
        } else if let Some(progress) = parse_progress(&line) {
            on_progress(progress);
        }
    };

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        if let Ok(line) = receiver.recv_timeout(Duration::from_millis(100)) {
            handle(line, &mut outcome);
        }
        if let Ok(Some(status)) = child.try_wait() {
            break status;
        }
    };
    let text = errors.join().unwrap_or_default();
    // A leitura termina quando o processo fecha a saida; pega o que ainda estava na fila.
    while let Ok(line) = receiver.recv_timeout(Duration::from_millis(200)) {
        handle(line, &mut outcome);
    }

    if !status.success() {
        return Err(Error::Message(friendly_error(&text)));
    }
    if outcome.downloaded + outcome.skipped == 0 {
        return Err(Error::Message("err.ytNoFile".into()));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("frey-yt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// yt-dlp falso: um .cmd que recebe o destino em %2 e segue o roteiro do teste.
    fn fake(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("yt-dlp-falso.cmd");
        std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
        path
    }

    const URL: &str = "https://www.youtube.com/watch?v=abc123";

    #[test]
    fn progresso_com_campos_ausentes_e_titulo_com_barra() {
        let p = parse_progress("FREYPROG|1048576|NA|2097152.0|NA|NA|Título | com barra").unwrap();
        assert_eq!((p.done, p.total, p.files_done, p.files_total), (1048576, Some(2097152), 0, 1));
        assert_eq!(p.file, "Título | com barra");
        let q = parse_progress("FREYPROG|10|100|NA|3|12|x").unwrap();
        assert_eq!((q.total, q.files_done, q.files_total), (Some(100), 2, 12));
        assert!(parse_progress("[download] outra coisa").is_none());
        assert!(parse_progress("FREYPROG|NA|NA|NA|NA|NA|x").is_none());
    }

    #[test]
    fn padrao_e_h264_em_mp4_para_edicao() {
        for quality in ["1080", "qualquer"] {
            assert_eq!(quality_args(quality)[1], "vcodec:h264,res:1080,acodec:aac");
        }
        assert!(quality_args("720")[1].contains("res:720"));
        assert!(quality_args("audio").contains(&"-x"));
        assert!(!quality_args("best").contains(&"--merge-output-format"));
        assert_eq!(quality_args("edit4k")[1], "hdr:sdr,res:2160,fps");
    }

    #[test]
    fn argumentos_protegem_contra_sobrescrita_e_link_como_opcao() {
        let args = build_args(URL, Path::new("D:\\p"), "1080");
        assert_eq!(&args[..2], ["-P", "D:\\p"]);
        for needed in ["--no-overwrites", "--continue", "--ignore-config", "--no-playlist", "--no-simulate"] {
            assert!(args.iter().any(|a| a == needed), "{needed}");
        }
        assert!(!args.iter().any(|a| a.contains("cookies")));
        assert_eq!(&args[args.len() - 2..], ["--", URL]);
    }

    #[test]
    fn erros_comuns_em_portugues() {
        assert_eq!(friendly_error("WARNING: x\nERROR: [youtube] a: Sign in to confirm you're not a bot"), "err.ytLogin");
        assert_eq!(friendly_error("ERROR: Private video"), "err.ytPrivate");
        assert_eq!(friendly_error("ERROR: [youtube] a: Video unavailable"), "err.ytUnavailable");
        assert_eq!(friendly_error("ERROR: This video is DRM protected"), "err.ytDrm");
        assert_eq!(friendly_error("ERROR: ffmpeg not found"), "err.ffmpegMissing");
        assert_eq!(friendly_error("ERROR: outro motivo"), "outro motivo");
        assert_eq!(friendly_error(""), "err.ytFailed");
    }

    #[test]
    fn baixa_e_informa_arquivo_e_progresso() {
        let dir = temp_dir("ok");
        let tool = fake(&dir, "echo FREYPROG^|50^|100^|NA^|NA^|NA^|Clipe\r\necho x> \"%~2\\Clipe.mp4\"\r\necho FREYDONE^|%~2\\Clipe.mp4");
        let dest = dir.join("saida");
        let mut seen = Vec::new();
        let outcome = run(&tool, URL, &dest, "1080", &AtomicBool::new(false), &mut |p| seen.push(p.done)).unwrap();
        assert_eq!((outcome.downloaded, outcome.skipped), (1, 0));
        assert_eq!(outcome.files, vec![dest.join("Clipe.mp4")]);
        assert_eq!(seen, vec![50]);
    }

    #[test]
    fn arquivo_que_ja_existia_conta_como_pulado() {
        let dir = temp_dir("skip");
        let dest = dir.join("saida");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("Clipe.mp4"), "original").unwrap();
        let tool = fake(&dir, "echo FREYDONE^|%~2\\Clipe.mp4");
        let outcome = run(&tool, URL, &dest, "1080", &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!((outcome.downloaded, outcome.skipped), (0, 1));
        assert!(outcome.files.is_empty());
    }

    #[test]
    fn falha_vira_mensagem_e_sem_arquivo_tambem() {
        let dir = temp_dir("fail");
        let tool = fake(&dir, "echo ERROR: Private video 1>&2\r\nexit /b 1");
        let result = run(&tool, URL, &dir.join("a"), "1080", &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytPrivate".into()));
        let silent = fake(&dir, "exit /b 0");
        let result = run(&silent, URL, &dir.join("b"), "1080", &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytNoFile".into()));
    }

    #[test]
    fn cancelar_encerra_a_arvore_de_processos() {
        let dir = temp_dir("cancel");
        let tool = fake(&dir, "echo FREYPROG^|1^|100^|NA^|NA^|NA^|Clipe\r\nping -n 30 127.0.0.1 > nul\r\necho FREYDONE^|%~2\\nunca.mp4");
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let result = run(&tool, URL, &dir.join("s"), "1080", &cancel, &mut |_| cancel.store(true, Ordering::Relaxed));
        assert_eq!(result.unwrap_err(), Error::Cancelled);
        assert!(started.elapsed().as_secs() < 15, "nao esperou o processo terminar sozinho");
    }

    #[test]
    fn sem_o_programa_explica_como_instalar() {
        let dir = temp_dir("missing");
        let result = run(&dir.join("nao-existe.exe"), URL, &dir, "1080", &AtomicBool::new(false), &mut |_| {});
        assert!(matches!(result, Err(Error::Message(m)) if m.starts_with("err.ytStart")));
        assert_eq!(MISSING, "err.ytMissing");
    }
}
