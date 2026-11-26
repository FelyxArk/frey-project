//! Download de YouTube, X, Instagram e TikTok pelo yt-dlp instalado no PATH.
//! Esses sites mudam com frequencia; quem acompanha isso e o yt-dlp, nao este app.
//! Nunca sobrescreve (--no-overwrites) e retoma parciais. So usa a conta do usuario se ele ligar
//! o login nas Configuracoes, so no YouTube e no Instagram, e so quando o site recusa sem login.

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
    /// None quando desligado ou incompleto (arquivo ainda nao escolhido). "auto" e resolvido fora daqui.
    pub fn from_settings(source: &str, file: &str) -> Option<Login> {
        match source {
            "firefox" | "brave" | "safari" | "chrome" | "edge" | "opera" | "vivaldi" | "chromium" => Some(Login::Browser(source.to_string())),
            "file" if !file.is_empty() => Some(Login::File(PathBuf::from(file))),
            _ => None,
        }
    }

    /// Nome curto para lembrar e mostrar: "firefox", "brave"... ou "file" para um cookies.txt.
    pub fn source(&self) -> String {
        match self {
            Login::Browser(name) => name.clone(),
            Login::File(_) => "file".into(),
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

/// "youtube" ou "instagram" quando o link pode usar o login opcional; X e TikTok nunca usam a conta.
pub fn site(url: &str) -> Option<&'static str> {
    if is_youtube(url) {
        Some("youtube")
    } else if url.contains("instagram.com/") {
        Some("instagram")
    } else {
        None
    }
}

/// Navegadores com perfil neste computador, na ordem em que vale procurar o login.
/// Firefox primeiro: no Windows, Chrome, Edge e parentes protegem o login contra outros programas.
pub fn installed_browsers() -> Vec<&'static str> {
    let home = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let mut found = Vec::new();
    #[cfg(windows)]
    {
        let (roaming, local) = (home("APPDATA"), home("LOCALAPPDATA"));
        let checks: [(&str, Option<PathBuf>); 7] = [
            ("firefox", roaming.as_ref().map(|r| r.join("Mozilla\\Firefox\\Profiles"))),
            ("brave", local.as_ref().map(|l| l.join("BraveSoftware\\Brave-Browser\\User Data"))),
            ("opera", roaming.as_ref().map(|r| r.join("Opera Software\\Opera Stable"))),
            ("vivaldi", local.as_ref().map(|l| l.join("Vivaldi\\User Data"))),
            ("chrome", local.as_ref().map(|l| l.join("Google\\Chrome\\User Data"))),
            ("edge", local.as_ref().map(|l| l.join("Microsoft\\Edge\\User Data"))),
            ("chromium", local.as_ref().map(|l| l.join("Chromium\\User Data"))),
        ];
        for (name, path) in checks {
            if path.is_some_and(|p| p.is_dir()) {
                found.push(name);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let lib = home("HOME").map(|h| h.join("Library"));
        let checks: [(&str, &str); 7] = [
            ("safari", "Cookies"),
            ("firefox", "Application Support/Firefox/Profiles"),
            ("chrome", "Application Support/Google/Chrome"),
            ("brave", "Application Support/BraveSoftware/Brave-Browser"),
            ("edge", "Application Support/Microsoft Edge"),
            ("opera", "Application Support/com.operasoftware.Opera"),
            ("vivaldi", "Application Support/Vivaldi"),
        ];
        for (name, rel) in checks {
            if lib.as_ref().is_some_and(|l| l.join(rel).exists()) {
                found.push(name);
            }
        }
    }
    found
}

/// Sites onde o login opcional pode entrar, com as mensagens de cada um. X e TikTok nunca usam a conta.
fn login_site(url: &str) -> Option<(&'static str, &'static str)> {
    if is_youtube(url) {
        Some(("err.ytLoginOff", "err.ytLoginRefused"))
    } else if url.contains("instagram.com/") {
        Some(("err.igLoginOff", "err.igLoginRefused"))
    } else {
        None
    }
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
        // Acima de 1080p o YouTube nao tem H.264: 2K e 4K vem em VP9 (AV1 so se nao houver VP9), em SDR.
        // A altura e um teto: video sem 4K vem na maior resolucao que tiver.
        "2160" => vec!["-S", "res:2160,fps,hdr:sdr,vcodec:vp9,acodec:aac", "--merge-output-format", "mp4"],
        "1440" => vec!["-S", "res:1440,fps,hdr:sdr,vcodec:vp9,acodec:aac", "--merge-output-format", "mp4"],
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

/// Pedidos que nao sao de qualidade: trazer a legenda junto. O trecho e cortado depois, no PC:
/// o --download-sections do yt-dlp fica parado lendo o video direto do YouTube.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra {
    /// Idiomas da legenda, como o yt-dlp entende ("pt.*,en.*"); None nao baixa legenda.
    pub subs: Option<String>,
}

/// "1:02:05" para o nome do arquivo e para o yt-dlp.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
fn build_args(url: &str, dest: &Path, quality: &str, skip_hls: bool, login: Option<&Login>) -> Vec<String> {
    build_args_with(url, dest, quality, skip_hls, login, &Extra::default())
}

fn build_args_with(url: &str, dest: &Path, quality: &str, skip_hls: bool, login: Option<&Login>, extra: &Extra) -> Vec<String> {
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
    if let Some(langs) = &extra.subs {
        args.extend(
            ["--write-subs", "--write-auto-subs", "--sub-langs", langs.as_str(), "--convert-subs", "srt"].map(String::from),
        );
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
    } else if lower.contains("video unavailable") || lower.contains("is unavailable") || lower.contains("not available") {
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

/// Baixa com trecho e legenda, procurando o login nas contas da lista, na ordem, so se o site pedir.
/// `on_login` recebe o login que funcionou, para ser lembrado.
#[allow(clippy::too_many_arguments)]
pub fn download_with(
    url: &str,
    dest: &Path,
    quality: &str,
    extra: &Extra,
    logins: &[Login],
    on_login: &mut dyn FnMut(&Login),
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<Outcome, Error> {
    let tool = find_tool("yt-dlp").ok_or_else(|| Error::Message(MISSING.into()))?;
    run_with(&tool, url, dest, quality, extra, logins, on_login, cancel, on_progress)
}

const LOGIN_ASKED: &str = "err.ytLogin";
const COOKIES_READ: &str = "err.ytCookiesRead";

#[cfg(test)]
fn run(tool: &Path, url: &str, dest: &Path, quality: &str, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    let logins: Vec<Login> = login.into_iter().cloned().collect();
    run_with(tool, url, dest, quality, &Extra::default(), &logins, &mut |_| {}, cancel, on_progress)
}

/// Sempre tenta sem a conta. So se o YouTube ou o Instagram recusar por falta de login, tenta as contas
/// da lista, uma por vez: a conta so e usada quando nao ha outro jeito.
#[allow(clippy::too_many_arguments)]
fn run_with(
    tool: &Path,
    url: &str,
    dest: &Path,
    quality: &str,
    extra: &Extra,
    logins: &[Login],
    on_login: &mut dyn FnMut(&Login),
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<Outcome, Error> {
    let first = run_once(tool, url, dest, quality, extra, None, cancel, on_progress);
    let Some((off, refused)) = login_site(url) else { return first };
    if !matches!(&first, Err(Error::Message(m)) if m == LOGIN_ASKED) {
        return first;
    }
    with_logins(logins, off, refused, |login| run_once(tool, url, dest, quality, extra, Some(login), cancel, on_progress), on_login)
}

/// Tenta cada login ate um funcionar. Login que nao pode ser lido (navegador protegido, arquivo que sumiu)
/// passa para o proximo; recusa do site com login tambem. Outro erro para na hora.
fn with_logins<T>(
    logins: &[Login],
    off: &str,
    refused: &str,
    mut try_one: impl FnMut(&Login) -> Result<T, Error>,
    on_login: &mut dyn FnMut(&Login),
) -> Result<T, Error> {
    if logins.is_empty() {
        return Err(Error::Message(off.into()));
    }
    let mut was_refused = false;
    for login in logins {
        if let Login::File(path) = login {
            if !path.is_file() {
                continue;
            }
        }
        match try_one(login) {
            Ok(value) => {
                on_login(login);
                return Ok(value);
            }
            Err(Error::Message(m)) if m == LOGIN_ASKED => was_refused = true,
            Err(Error::Message(m)) if m == COOKIES_READ => {}
            other => return other,
        }
    }
    Err(Error::Message(if was_refused { refused } else { COOKIES_READ }.into()))
}

#[allow(clippy::too_many_arguments)]
fn run_once(tool: &Path, url: &str, dest: &Path, quality: &str, extra: &Extra, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    match attempt(tool, url, dest, quality, extra, is_youtube(url), login, cancel, on_progress) {
        Err(Error::Message(message)) if message == RETRY_WITH_HLS => attempt(tool, url, dest, quality, extra, false, login, cancel, on_progress),
        other => other,
    }
}

// ---- Previa do link: capa, titulo, duracao, capitulos e, numa playlist, os videos dela.

#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub title: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub title: String,
    pub uploader: String,
    pub duration: Option<f64>,
    pub thumbnail: String,
    /// Maior altura de video oferecida (2160 = 4K).
    pub height: Option<u64>,
    pub chapters: Vec<Chapter>,
    /// Videos da playlist; vazio quando o link e de um video so.
    pub entries: Vec<Entry>,
    pub has_subs: bool,
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// A maior imagem da lista "thumbnails", ou o campo "thumbnail".
fn best_thumb(value: &serde_json::Value) -> String {
    let listed = value.get("thumbnails").and_then(|t| t.as_array()).and_then(|list| {
        list.iter()
            .filter(|t| t.get("url").and_then(|u| u.as_str()).is_some_and(|u| u.starts_with("http")))
            .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0) * t.get("height").and_then(|h| h.as_u64()).unwrap_or(0))
            .map(|t| text(t, "url"))
    });
    listed.filter(|u| !u.is_empty()).unwrap_or_else(|| text(value, "thumbnail"))
}

/// Le o JSON do yt-dlp (-J --flat-playlist).
pub fn parse_meta(json: &serde_json::Value) -> Meta {
    let entries = json
        .get("entries")
        .and_then(|e| e.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let id = text(e, "id");
                    let mut url = text(e, "url");
                    if !url.starts_with("http") {
                        url = text(e, "webpage_url");
                    }
                    if !url.starts_with("http") && !id.is_empty() && text(e, "ie_key").eq_ignore_ascii_case("youtube") {
                        url = format!("https://www.youtube.com/watch?v={id}");
                    }
                    url.starts_with("http").then(|| Entry { url, title: text(e, "title"), duration: e.get("duration").and_then(|d| d.as_f64()), thumbnail: best_thumb(e) })
                })
                .collect()
        })
        .unwrap_or_default();
    let chapters = json
        .get("chapters")
        .and_then(|c| c.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|c| Some(Chapter { title: text(c, "title"), start: c.get("start_time")?.as_f64()?, end: c.get("end_time")?.as_f64()? }))
                .collect()
        })
        .unwrap_or_default();
    let height = json
        .get("formats")
        .and_then(|f| f.as_array())
        .and_then(|list| list.iter().filter_map(|f| f.get("height").and_then(|h| h.as_u64())).max())
        .or_else(|| json.get("height").and_then(|h| h.as_u64()));
    let has_subs = ["subtitles", "automatic_captions"].iter().any(|k| json.get(*k).and_then(|s| s.as_object()).is_some_and(|m| !m.is_empty()));
    let uploader = [text(json, "uploader"), text(json, "channel"), text(json, "uploader_id")].into_iter().find(|u| !u.is_empty()).unwrap_or_default();
    Meta { title: text(json, "title"), uploader, duration: json.get("duration").and_then(|d| d.as_f64()), thumbnail: best_thumb(json), height, chapters, entries, has_subs }
}

fn probe_once(tool: &Path, url: &str, login: Option<&Login>) -> Result<Meta, Error> {
    let mut command = Command::new(tool);
    crate::platform::quiet(&mut command);
    command.args(["--ignore-config", "-J", "--flat-playlist", "--no-warnings", "--encoding", "utf-8", "--socket-timeout", "20"]);
    // Link de video dentro de playlist mostra so o video, como o download faz.
    if url.contains("watch?") || url.contains("youtu.be/") || url.contains("/shorts/") {
        command.arg("--no-playlist");
    }
    if let Some(login) = login {
        command.args(login.args());
    }
    let output = command.args(["--", url]).env("PYTHONUTF8", "1").output().map_err(|e| Error::Message(crate::i18n::code("err.ytStart", &[&e.to_string()])))?;
    if !output.status.success() {
        return Err(Error::Message(friendly_error(&String::from_utf8_lossy(&output.stderr))));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| Error::Message("err.ytFailed".into()))?;
    Ok(parse_meta(&json))
}

/// Previa de um link, com a mesma regra de login do download. Devolve tambem o login que funcionou.
pub fn probe(url: &str, logins: &[Login]) -> Result<(Meta, Option<Login>), Error> {
    let tool = find_tool("yt-dlp").ok_or_else(|| Error::Message(MISSING.into()))?;
    probe_with(&tool, url, logins)
}

fn probe_with(tool: &Path, url: &str, logins: &[Login]) -> Result<(Meta, Option<Login>), Error> {
    let first = probe_once(tool, url, None);
    let Some((off, refused)) = login_site(url) else { return first.map(|m| (m, None)) };
    match first {
        Err(Error::Message(m)) if m == LOGIN_ASKED => {}
        other => return other.map(|m| (m, None)),
    }
    let mut used = None;
    let meta = with_logins(logins, off, refused, |login| probe_once(tool, url, Some(login)), &mut |login| used = Some(login.clone()))?;
    Ok((meta, used))
}

#[allow(clippy::too_many_arguments)]
fn attempt(tool: &Path, url: &str, dest: &Path, quality: &str, extra: &Extra, skip_hls: bool, login: Option<&Login>, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
    std::fs::create_dir_all(dest)?;
    let before = existing_files(dest);
    let mut command = Command::new(tool);
    crate::platform::quiet(&mut command);
    crate::platform::own_group(&mut command);
    let mut child = command
        .args(build_args_with(url, dest, quality, skip_hls, login, extra))
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
    fn login_tambem_vale_no_instagram_e_so_quando_ele_pede() {
        let dir = temp_dir("login-ig");
        let log = dir.join("chamadas.txt");
        // Mensagem real do yt-dlp quando o Instagram esconde o post de quem nao esta logado.
        let script = format!(
            "echo %* >> \"{}\"\r\necho %* | findstr /c:\"--cookies-from-browser\" >nul || (echo ERROR: [Instagram] a: Instagram sent an empty media response. Check if this post is accessible in your browser without being logged-in. If it is not, then use --cookies-from-browser or --cookies for the authentication. 1>&2 & exit /b 1)\r\necho x> \"%~2\\Reel [a].mp4\"\r\necho FREYDONE^|%~2\\Reel [a].mp4",
            log.display()
        );
        let tool = fake(&dir, &script);
        let reel = "https://www.instagram.com/reel/a/";
        let off = run(&tool, reel, &dir.join("s"), "1080", None, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(off.unwrap_err(), Error::Message("err.igLoginOff".into()));
        std::fs::remove_file(&log).unwrap();
        let firefox = Login::Browser("firefox".into());
        let outcome = run(&tool, reel, &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.downloaded, 1);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().count(), 2, "primeiro sem a conta, depois com ela");
        assert!(!calls.lines().next().unwrap().contains("cookies-from-browser"));
        assert!(calls.lines().nth(1).unwrap().contains("--cookies-from-browser firefox"));
        let refuses = fake(&dir, "echo ERROR: [Instagram] a: Instagram sent an empty media response. use --cookies 1>&2\r\nexit /b 1");
        let result = run(&refuses, reel, &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.igLoginRefused".into()), "recusou mesmo com a conta");
    }

    #[test]
    fn login_nunca_vai_para_x_e_tiktok_nem_para_quem_nao_pediu() {
        let dir = temp_dir("login-social");
        let log = dir.join("chamadas.txt");
        let tool = fake(&dir, &format!("echo %* >> \"{}\"\r\necho ERROR: [twitter] a: login required, use --cookies 1>&2\r\nexit /b 1", log.display()));
        let firefox = Login::Browser("firefox".into());
        for url in ["https://x.com/a/status/1", "https://www.tiktok.com/@a/video/1"] {
            let result = run(&tool, url, &dir.join("s"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {});
            assert_eq!(result.unwrap_err(), Error::Message("err.ytLogin".into()), "{url}");
        }
        assert!(!std::fs::read_to_string(&log).unwrap().contains("cookies-from-browser"));
        let ok = fake(&dir, &format!("echo %* >> \"{}\"\r\necho x> \"%~2\\Clipe.mp4\"\r\necho FREYDONE^|%~2\\Clipe.mp4", log.display()));
        std::fs::remove_file(&log).unwrap();
        for url in [URL, "https://www.instagram.com/reel/a/"] {
            run(&ok, url, &dir.join("t"), "1080", Some(&firefox), &AtomicBool::new(false), &mut |_| {}).unwrap();
        }
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
        assert_eq!(Login::from_settings("safari-x", ""), None, "nome desconhecido nao vira login");
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

    #[test]
    fn login_automatico_tenta_cada_conta_e_avisa_qual_funcionou() {
        let dir = temp_dir("auto");
        let log = dir.join("chamadas.txt");
        // So o login do Firefox abre o post; o Brave e o Chrome nao tem a conta.
        let script = format!(
            "echo %* >> \"{}\"\r\necho %* | findstr /c:\"--cookies-from-browser firefox\" >nul || (echo ERROR: [Instagram] a: login required, use --cookies 1>&2 & exit /b 1)\r\necho x> \"%~2\\Reel [a].mp4\"\r\necho FREYDONE^|%~2\\Reel [a].mp4",
            log.display()
        );
        let tool = fake(&dir, &script);
        let logins = vec![
            Login::File(dir.join("login-frey-que-nao-existe.txt")),
            Login::Browser("brave".into()),
            Login::Browser("firefox".into()),
            Login::Browser("chrome".into()),
        ];
        let mut used = None;
        let outcome = run_with(&tool, "https://www.instagram.com/reel/a/", &dir.join("s"), "1080", &Extra::default(), &logins, &mut |l| used = Some(l.clone()), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.downloaded, 1);
        assert_eq!(used, Some(Login::Browser("firefox".into())));
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().count(), 3, "sem conta, Brave, Firefox; o arquivo que sumiu e o Chrome nem sao tentados");
        assert!(!calls.contains("chrome"));
    }

    #[test]
    fn navegador_protegido_passa_para_o_proximo() {
        let dir = temp_dir("protegido");
        let script = "echo %* | findstr /c:\"--cookies-from-browser chrome\" >nul && (echo ERROR: Failed to decrypt with DPAPI 1>&2 & exit /b 1)\r\necho %* | findstr /c:\"--cookies-from-browser\" >nul || (echo ERROR: [youtube] a: Sign in to confirm your age 1>&2 & exit /b 1)\r\necho x> \"%~2\\Clipe.mp4\"\r\necho FREYDONE^|%~2\\Clipe.mp4";
        let tool = fake(&dir, script);
        let logins = vec![Login::Browser("chrome".into()), Login::Browser("opera".into())];
        let mut used = None;
        run_with(&tool, URL, &dir.join("s"), "1080", &Extra::default(), &logins, &mut |l| used = Some(l.clone()), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(used, Some(Login::Browser("opera".into())));
        // Nenhum navegador pode ser lido: explica que nao achou o login, nao que o site recusou.
        let only_chrome = vec![Login::Browser("chrome".into())];
        let result = run_with(&tool, URL, &dir.join("s"), "1080", &Extra::default(), &only_chrome, &mut |_| {}, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(result.unwrap_err(), Error::Message("err.ytCookiesRead".into()));
    }

    #[test]
    fn legenda_vira_argumentos() {
        let extra = Extra { subs: Some("pt.*,en.*".into()) };
        let args = build_args_with(URL, Path::new("D:\\p"), "1080", true, None, &extra);
        assert!(args.windows(2).any(|w| w == ["--sub-langs", "pt.*,en.*"]));
        assert!(args.iter().any(|a| a == "--write-auto-subs") && args.windows(2).any(|w| w == ["--convert-subs", "srt"]));
        let plain = build_args_with(URL, Path::new("D:\\p"), "1080", true, None, &Extra::default());
        assert!(!plain.iter().any(|a| a == "--write-subs" || a == "--download-sections"));
        assert_eq!(clock(59.6), "1:00");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn previa_le_titulo_capitulos_altura_e_playlist() {
        let video: serde_json::Value = serde_json::from_str(r#"{
            "title": "Tokyo at Night", "uploader": "Rambalac", "duration": 768.0,
            "thumbnails": [{"url": "https://i.ytimg.com/a.jpg", "width": 120, "height": 90}, {"url": "https://i.ytimg.com/b.jpg", "width": 1280, "height": 720}],
            "formats": [{"height": 1080}, {"height": 2160}, {"height": null}],
            "chapters": [{"title": "Shinjuku", "start_time": 0.0, "end_time": 170.0}, {"title": "Kabukicho", "start_time": 170.0, "end_time": 415.0}],
            "subtitles": {}, "automatic_captions": {"en": []}
        }"#).unwrap();
        let meta = parse_meta(&video);
        assert_eq!((meta.title.as_str(), meta.uploader.as_str(), meta.duration), ("Tokyo at Night", "Rambalac", Some(768.0)));
        assert_eq!(meta.thumbnail, "https://i.ytimg.com/b.jpg");
        assert_eq!(meta.height, Some(2160));
        assert_eq!(meta.chapters[1], Chapter { title: "Kabukicho".into(), start: 170.0, end: 415.0 });
        assert!(meta.has_subs && meta.entries.is_empty());

        let playlist: serde_json::Value = serde_json::from_str(r#"{
            "_type": "playlist", "title": "Stock Japão", "uploader": "Canal",
            "entries": [
                {"id": "aaa", "ie_key": "Youtube", "url": "https://www.youtube.com/watch?v=aaa", "title": "Um", "duration": 45, "thumbnails": [{"url": "https://i.ytimg.com/aaa.jpg"}]},
                {"id": "bbb", "ie_key": "Youtube", "url": "bbb", "title": "Dois"},
                {"id": "ccc", "title": "Sem link"}
            ]
        }"#).unwrap();
        let list = parse_meta(&playlist);
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.entries[0], Entry { url: "https://www.youtube.com/watch?v=aaa".into(), title: "Um".into(), duration: Some(45.0), thumbnail: "https://i.ytimg.com/aaa.jpg".into() });
        assert_eq!(list.entries[1].url, "https://www.youtube.com/watch?v=bbb");
        assert!(!list.has_subs);
    }

    #[test]
    fn previa_usa_o_login_so_quando_o_site_pede() {
        let dir = temp_dir("probe");
        let script = "echo %* | findstr /c:\"--cookies-from-browser firefox\" >nul || (echo ERROR: [Instagram] a: login required, use --cookies 1>&2 & exit /b 1)\r\necho {\"title\": \"Reel\", \"duration\": 12}";
        let tool = fake(&dir, script);
        let reel = "https://www.instagram.com/reel/a/";
        let (meta, used) = probe_with(&tool, reel, &[Login::Browser("brave".into()), Login::Browser("firefox".into())]).unwrap();
        assert_eq!((meta.title.as_str(), meta.duration), ("Reel", Some(12.0)));
        assert_eq!(used, Some(Login::Browser("firefox".into())));
        assert_eq!(probe_with(&tool, reel, &[]).unwrap_err(), Error::Message("err.igLoginOff".into()));
        let open = fake(&dir, "echo {\"title\": \"Publico\"}");
        assert_eq!(probe_with(&open, reel, &[Login::Browser("firefox".into())]).unwrap().1, None, "post aberto nao usa a conta");
    }

    #[test]
    fn dois_k_e_quatro_k_tem_teto_de_altura_e_preferem_vp9() {
        assert_eq!(quality_args("2160"), vec!["-S", "res:2160,fps,hdr:sdr,vcodec:vp9,acodec:aac", "--merge-output-format", "mp4"]);
        assert!(quality_args("1440")[1].starts_with("res:1440,"));
    }

    #[test]
    fn site_do_login() {
        assert_eq!(site(URL), Some("youtube"));
        assert_eq!(site("https://www.instagram.com/p/a/"), Some("instagram"));
        assert_eq!(site("https://x.com/a/status/1"), None);
        assert_eq!(Login::Browser("opera".into()).source(), "opera");
        assert_eq!(Login::from_settings("vivaldi", ""), Some(Login::Browser("vivaldi".into())));
        assert_eq!(Login::from_settings("auto", ""), None, "auto e resolvido pelo app");
    }
}
