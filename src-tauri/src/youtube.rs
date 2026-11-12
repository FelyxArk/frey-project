//! Download de YouTube, X, Instagram e TikTok pelo yt-dlp instalado no PATH.
//! Esses sites mudam com frequencia; quem acompanha isso e o yt-dlp, nao este app.
//! Nunca sobrescreve (--no-overwrites) e retoma parciais. So usa a conta do usuario se ele ligar
//! o login do YouTube nas Configuracoes, e mesmo assim so quando o YouTube recusa sem login.

use crate::convert::find_tool;
use crate::drive::{Error, Outcome, Progress};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub const MISSING: &str = "err.ytMissing";
const PROGRESS_PREFIX: &str = "FREYPROG|";
const DONE_PREFIX: &str = "FREYDONE|";

/// Login do YouTube escolhido nas Configuracoes: navegador de onde o yt-dlp le a sessao,
/// ou um arquivo cookies.txt exportado pelo usuario.
#[derive(Clone, Debug, PartialEq)]
pub enum Login {
    Browser(String),
    File(PathBuf),
}

impl Login {
    /// None quando desligado ou incompleto (arquivo ainda nao escolhido).
    pub fn from_settings(source: &str, file: &str) -> Option<Login> {
        match source {
            "firefox" | "brave" | "safari" | "chrome" => Some(Login::Browser(source.to_string())),
            "file" if !file.is_empty() => Some(Login::File(PathBuf::from(file))),
            _ => None,
        }
    }

    fn args(&self) -> [String; 2] {
        match self {
            Login::Browser(name) => ["--cookies-from-browser".into(), name.clone()],
            Login::File(path) => ["--cookies".into(), path.display().to_string()],
        }
    }
}

/// Tipos de link que este modulo baixa.
pub fn handles(kind: &str) -> bool {
    matches!(kind, "youtube" | "x" | "instagram" | "tiktok")
}

fn is_youtube(url: &str) -> bool {
    url.contains("youtube.com/") || url.contains("youtu.be/")
}

/// Nas redes sociais o titulo pode ser um texto longo ou se repetir ("Video by autor").
/// O ID impede que um post seja pulado por ter o mesmo nome de outro.
fn name_template(url: &str) -> &'static str {
    if is_youtube(url) {
        "%(title)s.%(ext)s"
    } else {
        "%(title).80B [%(id)s].%(ext)s"
    }
}

/// No TikTok o formato "audio" e so a musica do post, muitas vezes um trecho: o som do video vem do proprio video.
const AUDIO_FORMAT: &str = "ba[format_id!=audio]/b";

/// Argumentos da qualidade escolhida. H.264/AAC por padrao: e o que entra direto na edicao.
pub fn quality_args(quality: &str) -> Vec<&'static str> {
    match quality {
        "720" => vec!["-S", "vcodec:h264,res:720,acodec:aac", "--merge-output-format", "mp4"],
        "best" => vec!["-S", "res,fps"],
        // Maior resolucao ate 4K, em SDR: o app reencoda depois para H.264 de 8 bits, e HDR ficaria com a cor errada.
        "edit4k" => vec!["-S", "hdr:sdr,res:2160,fps", "--merge-output-format", "mkv"],
        "audio" => vec!["-f", AUDIO_FORMAT, "-x", "--audio-format", "m4a", "--audio-quality", "0"],
        "audio-mp3" => vec!["-f", AUDIO_FORMAT, "-x", "--audio-format", "mp3", "--audio-quality", "0"],
        "audio-wav" => vec!["-f", AUDIO_FORMAT, "-x", "--audio-format", "wav"],
        "audio-flac" => vec!["-f", AUDIO_FORMAT, "-x", "--audio-format", "flac"],
        "audio-opus" => vec!["-f", AUDIO_FORMAT, "-x", "--audio-format", "opus", "--audio-quality", "0"],
        _ => vec!["-S", "vcodec:h264,res:1080,acodec:aac", "--merge-output-format", "mp4"],
    }
}

/// Pular a lista HLS economiza cerca de 1 s e nao muda o formato escolhido em videos comuns.
/// Transmissoes ao vivo precisam dela: nesse caso o download tenta de novo sem o atalho.
const SKIP_HLS: &str = "youtube:skip=hls";
const RETRY_WITH_HLS: &str = "retry-with-hls";

fn needs_hls(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("no video formats found") || lower.contains("requested format is not available") || lower.contains("live stream")
}

fn build_args(url: &str, dest: &Path, quality: &str, skip_hls: bool, login: Option<&Login>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-P".into(), dest.display().to_string()];
    let fixed = [
        "--ignore-config", "--no-playlist", "--no-overwrites", "--continue", "--newline", "--encoding", "utf-8",
        "-o", name_template(url),
        "--no-simulate", "--progress",
        "--progress-template",
        "download:FREYPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(info.playlist_index)s|%(info.n_entries)s|%(info.title)s",
        "--print", "after_move:FREYDONE|%(filepath)s",
    ];
    args.extend(fixed.iter().map(|s| s.to_string()));
    args.extend(quality_args(quality).iter().map(|s| s.to_string()));
    // Sem isto, extrair audio apagaria um arquivo que ja existia com o mesmo nome (ex.: o .m4a de antes).
    if quality.starts_with("audio") {
        args.push("--keep-video".to_string());
    }
    if skip_hls {
        args.extend(["--extractor-args".to_string(), SKIP_HLS.to_string()]);
    }
    if let Some(login) = login {
        args.extend(login.args());
    }
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
    // Falha ao ler o login do navegador ou o arquivo: vem antes, porque tambem fala em cookies.
    if lower.contains("cookies database") || lower.contains("cookie database") || lower.contains("netscape format") || lower.contains("failed to decrypt") || lower.contains("could not copy") {
        return "err.ytCookiesRead".into();
    }
    if lower.contains("no video could be found") || lower.contains("no video in this") {
        "err.ytNoVideo".into()
    } else if lower.contains("ip address is blocked") || lower.contains("not available in your") {
        "err.ytBlocked".into()
    } else if lower.contains("sign in") || lower.contains("log in") || lower.contains("login") || lower.contains("cookies") {
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
    crate::platform::kill_tree(pid);
}

fn existing_files(dest: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(dest)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Apaga so o que este download criou e nao e o resultado: nunca um arquivo que ja estava la.
fn remove_intermediates(dest: &Path, before: &HashSet<PathBuf>, finals: &[PathBuf]) {
    let stems: HashSet<_> = finals.iter().filter_map(|p| p.file_stem().map(|s| s.to_os_string())).collect();
    for file in existing_files(dest) {
        let is_ours = file.file_stem().is_some_and(|stem| stems.contains(stem));
        if is_ours && file.is_file() && !before.contains(&file) && !finals.contains(&file) {
            let _ = std::fs::remove_file(file);
        }
    }
}

pub fn download(url: &str, dest: &Path, quality: &str, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    let tool = find_tool("yt-dlp").ok_or_else(|| Error::Message(MISSING.into()))?;
    run(&tool, url, dest, quality, login, cancel, on_progress)
}

const LOGIN_ASKED: &str = "err.ytLogin";

/// Sempre tenta sem a conta. So se o YouTube recusar por falta de login, e o usuario tiver ligado
/// o login, tenta de novo com ele: a conta so e usada quando nao ha outro jeito.
fn run(tool: &Path, url: &str, dest: &Path, quality: &str, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    let first = run_once(tool, url, dest, quality, None, cancel, on_progress);
    if !matches!(&first, Err(Error::Message(m)) if m == LOGIN_ASKED) || !is_youtube(url) {
        return first;
    }
    let Some(login) = login else {
        return Err(Error::Message("err.ytLoginOff".into()));
    };
    if let Login::File(path) = login {
        if !path.is_file() {
            return Err(Error::Message("err.ytCookiesRead".into()));
        }
    }
    match run_once(tool, url, dest, quality, Some(login), cancel, on_progress) {
        Err(Error::Message(m)) if m == LOGIN_ASKED => Err(Error::Message("err.ytLoginRefused".into())),
        other => other,
    }
}

fn run_once(tool: &Path, url: &str, dest: &Path, quality: &str, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    match attempt(tool, url, dest, quality, is_youtube(url), login, cancel, on_progress) {
        Err(Error::Message(message)) if message == RETRY_WITH_HLS => attempt(tool, url, dest, quality, false, login, cancel, on_progress),
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
fn attempt(tool: &Path, url: &str, dest: &Path, quality: &str, skip_hls: bool, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    std::fs::create_dir_all(dest)?;
    let before = existing_files(dest);
    let mut command = Command::new(tool);
    crate::platform::quiet(&mut command);
    crate::platform::own_group(&mut command);
    let mut child = command
        .args(build_args(url, dest, quality, skip_hls, login))
        .env("PYTHONUTF8", "1")
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
    let mut finals: Vec<PathBuf> = Vec::new();
    let mut handle = |line: String, outcome: &mut Outcome| {
        if let Some(path) = line.strip_prefix(DONE_PREFIX) {
            let path = PathBuf::from(path.trim());
            finals.push(path.clone());
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
        if skip_hls && needs_hls(&text) {
            return Err(Error::Message(RETRY_WITH_HLS.into()));
        }
        return Err(Error::Message(friendly_error(&text)));
    }
    if quality.starts_with("audio") {
        remove_intermediates(dest, &before, &finals);
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
    fn opcoes_de_audio_pedem_o_formato_certo() {
        for (quality, format) in [("audio", "m4a"), ("audio-mp3", "mp3"), ("audio-wav", "wav"), ("audio-flac", "flac"), ("audio-opus", "opus")] {
            let args = quality_args(quality);
            assert!(args.contains(&"-x"), "{quality}");
            assert!(args.windows(2).any(|w| w == ["--audio-format", format]), "{quality}");
        }
    }

    #[test]
    fn extrair_audio_nao_apaga_arquivo_que_ja_existia() {
        let dir = temp_dir("keep");
        let dest = dir.join("saida");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("Clipe.m4a"), "meu arquivo").unwrap();
        std::fs::write(dest.join("Outro.webm"), "outro").unwrap();
        let tool = fake(&dir, "echo x> \"%~2\\Clipe.webm\"\r\necho x> \"%~2\\Clipe.mp3\"\r\necho FREYDONE^|%~2\\Clipe.mp3");
        let outcome = run(&tool, URL, &dest, "audio-mp3", None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.files, vec![dest.join("Clipe.mp3")]);
        assert_eq!(std::fs::read_to_string(dest.join("Clipe.m4a")).unwrap(), "meu arquivo");
        assert!(dest.join("Outro.webm").exists(), "arquivo de outro video fica");
        assert!(!dest.join("Clipe.webm").exists(), "intermediario criado agora sai");
    }

    #[test]
    fn audio_nao_usa_a_musica_do_tiktok() {
        for quality in ["audio", "audio-mp3", "audio-wav", "audio-flac", "audio-opus"] {
            assert!(quality_args(quality).contains(&"ba[format_id!=audio]/b"), "{quality}");
        }
    }

    #[test]
    fn audio_pede_para_manter_e_video_nao() {
        assert!(build_args(URL, Path::new("D:\\p"), "audio-mp3", true, None).iter().any(|a| a == "--keep-video"));
        assert!(!build_args(URL, Path::new("D:\\p"), "1080", true, None).iter().any(|a| a == "--keep-video"));
    }

    #[test]
    fn sem_lista_hls_tenta_de_novo_com_ela() {
        let dir = temp_dir("hls");
        let script = "echo %* | findstr /c:\"skip=hls\" >nul && (echo ERROR: [youtube] No video formats found 1>&2 & exit /b 1)\r\necho x> \"%~2\\Live.mp4\"\r\necho FREYDONE^|%~2\\Live.mp4";
        let tool = fake(&dir, script);
        let dest = dir.join("saida");
        let outcome = run(&tool, URL, &dest, "1080", None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.files, vec![dest.join("Live.mp4")]);
    }

    #[test]
    fn outros_erros_nao_repetem_o_download() {
        let dir = temp_dir("noretry");
        let count = dir.join("tentativas.txt");
        let tool = fake(&dir, &format!("echo x>> \"{}\"\r\necho ERROR: Private video 1>&2\r\nexit /b 1", count.display()));
        let result = run(&tool, URL, &dir.join("s"), "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytPrivate".into()));
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
    }

    #[test]
    fn redes_sociais_usam_id_no_nome_e_nao_pedem_atalho_do_youtube() {
        let dir = temp_dir("social");
        let script = "echo %* | findstr /c:\"skip=hls\" >nul && (echo ERROR: usou atalho do YouTube 1>&2 & exit /b 1)\r\necho x> \"%~2\\Post [1].mp4\"\r\necho FREYDONE^|%~2\\Post [1].mp4";
        let tool = fake(&dir, script);
        let dest = dir.join("saida");
        let outcome = run(&tool, "https://www.tiktok.com/@a/video/1", &dest, "1080", None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.downloaded, 1);
        for url in ["https://x.com/a/status/1", "https://www.instagram.com/reel/a/", "https://vm.tiktok.com/a/"] {
            let args = build_args(url, Path::new("D:\\p"), "1080", false, None);
            assert!(args.iter().any(|a| a.contains("[%(id)s]")), "{url}");
            assert_eq!(args.last().unwrap(), url);
        }
        assert!(build_args(URL, Path::new("D:\\p"), "1080", true, None).iter().any(|a| a == "%(title)s.%(ext)s"));
    }

    #[test]
    fn tipos_baixados_pelo_ytdlp() {
        for kind in ["youtube", "x", "instagram", "tiktok"] {
            assert!(handles(kind), "{kind}");
        }
        assert!(!handles("file") && !handles("folder"));
    }

    #[test]
    fn login_so_entra_quando_o_youtube_pede() {
        let dir = temp_dir("login");
        let log = dir.join("chamadas.txt");
        // Recusa sem login; com --cookies-from-browser, baixa.
        let script = format!(
            "echo %* >> \"{}\"\r\necho %* | findstr /c:\"--cookies-from-browser\" >nul || (echo ERROR: [youtube] a: Sign in to confirm you're not a bot 1>&2 & exit /b 1)\r\necho x> \"%~2\\Clipe.mp4\"\r\necho FREYDONE^|%~2\\Clipe.mp4",
            log.display()
        );
        let tool = fake(&dir, &script);
        let firefox = Login::Browser("firefox".into());
        let outcome = run(&tool, URL, &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.downloaded, 1);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().count(), 2, "primeiro sem a conta, depois com ela");
        assert!(!calls.lines().next().unwrap().contains("cookies"));
        assert!(calls.lines().nth(1).unwrap().contains("--cookies-from-browser firefox"));
    }

    #[test]
    fn sem_login_ligado_explica_onde_ligar() {
        let dir = temp_dir("login-off");
        let tool = fake(&dir, "echo ERROR: [youtube] a: Sign in to confirm you're not a bot 1>&2\r\nexit /b 1");
        let result = run(&tool, URL, &dir.join("s"), "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytLoginOff".into()));
        let firefox = Login::Browser("firefox".into());
        let result = run(&tool, URL, &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytLoginRefused".into()), "recusou mesmo com a conta");
    }

    #[test]
    fn login_nunca_vai_para_outros_sites_nem_para_quem_nao_pediu() {
        let dir = temp_dir("login-social");
        let log = dir.join("chamadas.txt");
        let tool = fake(&dir, &format!("echo %* >> \"{}\"\r\necho ERROR: [Instagram] a: login required, use --cookies 1>&2\r\nexit /b 1", log.display()));
        let firefox = Login::Browser("firefox".into());
        let result = run(&tool, "https://www.instagram.com/reel/a/", &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytLogin".into()));
        assert!(!std::fs::read_to_string(&log).unwrap().contains("cookies-from-browser"));
        let ok = fake(&dir, &format!("echo %* >> \"{}\"\r\necho x> \"%~2\\Clipe.mp4\"\r\necho FREYDONE^|%~2\\Clipe.mp4", log.display()));
        std::fs::remove_file(&log).unwrap();
        run(&ok, URL, &dir.join("t"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(!std::fs::read_to_string(&log).unwrap().contains("cookies"), "video que baixa sem login nunca usa a conta");
    }

    #[test]
    fn arquivo_de_login_que_sumiu_nao_e_passado_ao_ytdlp() {
        let dir = temp_dir("login-file");
        let tool = fake(&dir, "echo ERROR: [youtube] a: Sign in to confirm you're not a bot 1>&2\r\nexit /b 1");
        let file = Login::File(dir.join("nao-existe.txt"));
        let result = run(&tool, URL, &dir.join("s"), "1080", Some(&file), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytCookiesRead".into()));
    }

    #[test]
    fn configuracao_do_login() {
        assert_eq!(Login::from_settings("", "x"), None);
        assert_eq!(Login::from_settings("file", ""), None, "arquivo ainda nao escolhido");
        assert_eq!(Login::from_settings("firefox", ""), Some(Login::Browser("firefox".into())));
        assert_eq!(Login::from_settings("brave", ""), Some(Login::Browser("brave".into())));
        assert_eq!(Login::from_settings("edge", ""), None, "Edge e Chrome no Windows nao funcionam");
        assert_eq!(Login::File("C:\\c.txt".into()).args(), ["--cookies".to_string(), "C:\\c.txt".to_string()]);
        assert_eq!(friendly_error("ERROR: could not find firefox cookies database in C:\\x"), "err.ytCookiesRead");
        assert_eq!(friendly_error("ERROR: 'c.txt' does not look like a Netscape format cookies file"), "err.ytCookiesRead");
    }

    #[test]
    fn progresso_com_campos_ausentes_e_titulo_com_barra() {        let p = parse_progress("FREYPROG|1048576|NA|2097152.0|NA|NA|Título | com barra").unwrap();
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
        let args = build_args(URL, Path::new("D:\\p"), "1080", true, None);
        assert!(args.windows(2).any(|w| w == ["--extractor-args", SKIP_HLS]));
        assert!(!build_args(URL, Path::new("D:\\p"), "1080", false, None).iter().any(|a| a == SKIP_HLS));
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
        assert_eq!(friendly_error("ERROR: [twitter] 1: No video could be found in this tweet"), "err.ytNoVideo");
        assert_eq!(friendly_error("ERROR: [TikTok] 1: Your IP address is blocked from accessing this post"), "err.ytBlocked");
        assert_eq!(friendly_error("ERROR: [Instagram] a: Instagram sent an empty media response. use --cookies-from-browser"), "err.ytLogin");
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
        let outcome = run(&tool, URL, &dest, "1080", None, &AtomicBool::new(false), &mut |p| seen.push(p.done)).unwrap();
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
        let outcome = run(&tool, URL, &dest, "1080", None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!((outcome.downloaded, outcome.skipped), (0, 1));
        assert!(outcome.files.is_empty());
    }

    #[test]
    fn falha_vira_mensagem_e_sem_arquivo_tambem() {
        let dir = temp_dir("fail");
        let tool = fake(&dir, "echo ERROR: Private video 1>&2\r\nexit /b 1");
        let result = run(&tool, URL, &dir.join("a"), "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytPrivate".into()));
        let silent = fake(&dir, "exit /b 0");
        let result = run(&silent, URL, &dir.join("b"), "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytNoFile".into()));
    }

    #[test]
    fn cancelar_encerra_a_arvore_de_processos() {
        let dir = temp_dir("cancel");
        let tool = fake(&dir, "echo FREYPROG^|1^|100^|NA^|NA^|NA^|Clipe\r\nping -n 30 127.0.0.1 > nul\r\necho FREYDONE^|%~2\\nunca.mp4");
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let result = run(&tool, URL, &dir.join("s"), "1080", None, &cancel, &mut |_| cancel.store(true, Ordering::Relaxed));
        assert_eq!(result.unwrap_err(), Error::Cancelled);
        assert!(started.elapsed().as_secs() < 15, "nao esperou o processo terminar sozinho");
    }

    #[test]
    fn sem_o_programa_explica_como_instalar() {
        let dir = temp_dir("missing");
        let result = run(&dir.join("nao-existe.exe"), URL, &dir, "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert!(matches!(result, Err(Error::Message(m)) if m.starts_with("err.ytStart")));
        assert_eq!(MISSING, "err.ytMissing");
    }
}
