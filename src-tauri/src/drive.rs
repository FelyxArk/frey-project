//! Download de links publicos do Google Drive, sem dependencia de Python/gdown.
//! Nunca sobrescreve: arquivo existente e pulado; o parcial (.part) e retomado.

use crate::models::clean_segment;
use regex::Regex;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use ureq::ResponseExt;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const STALL_LIMIT: Duration = Duration::from_secs(60);
const NO_ACCESS: &str = "err.noAccess";

#[derive(Debug, PartialEq)]
pub enum Error {
    Cancelled,
    Message(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Message(crate::i18n::code("err.diskWrite", &[&e.to_string()]))
    }
}

fn msg<T>(text: &str) -> Result<T, Error> {
    Err(Error::Message(text.to_string()))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Link {
    pub url: String,
    pub kind: &'static str,
}

/// Extrai os links do Drive de um texto colado, sem repetir e ignorando outros sites.
pub fn parse_links(text: &str) -> Vec<Link> {
    let url = Regex::new(r#"https?://(?:(?:drive|docs)\.google\.com|(?:www\.|m\.|music\.)?youtube\.com|youtu\.be)/[^\s"'<>]+"#).unwrap();
    let bare = Regex::new(r"^[a-zA-Z0-9_-]{25,}$").unwrap();
    let mut found: Vec<String> = url.find_iter(text).map(|m| m.as_str().to_string()).collect();
    found.extend(text.lines().map(str::trim).filter(|l| bare.is_match(l)).map(String::from));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|l| seen.insert(l.clone()))
        .map(|url| {
            let kind = if url.contains("youtube.com/") || url.contains("youtu.be/") {
                "youtube"
            } else if url.contains("/folders/") {
                "folder"
            } else {
                "file"
            };
            Link { url, kind }
        })
        .collect()
}

pub fn extract_id(link: &str) -> Option<String> {
    let link = link.trim();
    for pattern in [r"/folders/([\w-]+)", r"/d/([\w-]+)", r"[?&]id=([\w-]+)"] {
        if let Some(c) = Regex::new(pattern).unwrap().captures(link) {
            return Some(c[1].to_string());
        }
    }
    Regex::new(r"^[\w-]{10,}$").unwrap().is_match(link).then(|| link.to_string())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let digits = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = digits.and_then(|d| u8::from_str_radix(d, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_entities(s: &str) -> String {
    let numeric = Regex::new(r"&#(x?)([0-9a-fA-F]+);").unwrap();
    let decoded = numeric.replace_all(s, |c: &regex::Captures| {
        let radix = if c[1].is_empty() { 10 } else { 16 };
        u32::from_str_radix(&c[2], radix).ok().and_then(char::from_u32).map(String::from).unwrap_or_default()
    });
    decoded.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Nome do arquivo no cabecalho Content-Disposition, ja seguro para o Windows.
pub fn filename_from_disposition(header: &str) -> Option<String> {
    let star = Regex::new(r"(?i)filename\*=UTF-8''([^;]+)").unwrap();
    let plain = Regex::new(r#"(?i)filename="([^"]+)""#).unwrap();
    let name = if let Some(c) = star.captures(header) {
        percent_decode(c[1].trim())
    } else {
        plain.captures(header)?[1].to_string()
    };
    let clean = clean_segment(&name);
    (!clean.is_empty()).then_some(clean)
}

/// Valor de cabecalho como texto. O Drive manda nomes acentuados em bytes crus (UTF-8 ou Latin-1).
pub fn decode_header(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Nome do arquivo na pagina de confirmacao: em arquivo grande, a resposta final vem sem nome.
pub fn name_from_confirmation(html: &str) -> Option<String> {
    let found = Regex::new(r#"class="uc-name-size"[^>]*>\s*<a[^>]*>([^<]+)</a>"#).unwrap().captures(html)?;
    let clean = clean_segment(&decode_entities(found[1].trim()));
    (!clean.is_empty()).then_some(clean)
}

/// URL de confirmacao que o Drive mostra em arquivos grandes demais para o antivirus.
pub fn confirmation_url(html: &str) -> Option<String> {
    let form = Regex::new(r#"(?s)<form[^>]*id="download-form"[^>]*>.*?</form>"#).unwrap().find(html)?.as_str();
    let action = Regex::new(r#"action="([^"]+)""#).unwrap().captures(form)?[1].to_string();
    let input = Regex::new(r#"<input[^>]*>"#).unwrap();
    let attr = |tag: &str, name: &str| {
        Regex::new(&format!(r#"\b{name}="([^"]*)""#)).unwrap().captures(tag).map(|c| c[1].to_string())
    };
    let params: Vec<String> = input
        .find_iter(form)
        .filter_map(|m| {
            let tag = m.as_str();
            if attr(tag, "type").as_deref() != Some("hidden") {
                return None;
            }
            Some(format!("{}={}", attr(tag, "name")?, attr(tag, "value")?))
        })
        .collect();
    Some(format!("{}?{}", decode_entities(&action), params.join("&")))
}

#[derive(Debug, PartialEq)]
pub enum Child {
    File(String, String),
    Folder(String, String),
}

/// Conteudo da pagina publica de pasta do Drive (nao e uma API oficial).
pub fn parse_folder_view(html: &str) -> Result<Vec<Child>, Error> {
    if !html.contains("<title>") {
        return msg("err.folderPage");
    }
    let anchor = Regex::new(r#"(?s)<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let file = Regex::new(r"^https://drive\.google\.com/file/d/([\w-]{25,})/view").unwrap();
    let doc = Regex::new(r"^https://docs\.google\.com/\w+/d/([\w-]{25,})/").unwrap();
    let folder = Regex::new(r"^https://drive\.google\.com/drive/folders/([\w-]{25,})").unwrap();
    let tags = Regex::new(r"<[^>]+>").unwrap();
    let mut children = Vec::new();
    for c in anchor.captures_iter(html) {
        let href = decode_entities(&c[1]);
        let name = decode_entities(tags.replace_all(&c[2], "").trim());
        if let Some(m) = file.captures(&href).or_else(|| doc.captures(&href)) {
            children.push(Child::File(m[1].to_string(), name));
        } else if let Some(m) = folder.captures(&href) {
            children.push(Child::Folder(m[1].to_string(), name));
        }
    }
    Ok(children)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub file: String,
    pub done: u64,
    pub total: Option<u64>,
    pub files_done: usize,
    pub files_total: usize,
}

#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub downloaded: usize,
    pub skipped: usize,
    /// Arquivos realmente baixados agora (os pulados nao entram).
    pub files: Vec<std::path::PathBuf>,
}

type Response = ureq::http::Response<ureq::Body>;
type ListedFile = (String, Option<String>, std::path::PathBuf);

const CONNECTION_FAILED: &str = "err.connFailed";
const CONNECTION_LOST: &str = "err.connLost";

fn header_text(response: &Response, name: &str) -> Option<String> {
    response.headers().get(name).map(|value| decode_header(value.as_bytes()))
}

pub struct Client {
    agent: ureq::Agent,
    stall_limit: Duration,
    host: String,
    docs_host: String,
    direct_host: String,
}

impl Client {
    pub fn new() -> Self {
        let mut client = Self::with_hosts("https://drive.google.com", "https://docs.google.com");
        client.direct_host = "https://drive.usercontent.google.com".into();
        client
    }

    fn with_hosts(host: &str, docs_host: &str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build()
            .into();
        Client { agent, stall_limit: STALL_LIMIT, host: host.into(), docs_host: docs_host.into(), direct_host: host.into() }
    }

    fn get(&self, url: &str, range_from: u64) -> Result<Response, ureq::Error> {
        let request = self.agent.get(url);
        if range_from > 0 {
            request.header("Range", &format!("bytes={range_from}-")).call()
        } else {
            request.call()
        }
    }

    fn get_checked(&self, url: &str) -> Result<Response, Error> {
        match self.get(url, 0) {
            Ok(response) if response.status().as_u16() < 400 => Ok(response),
            Ok(_) => msg(NO_ACCESS),
            Err(_) => msg(CONNECTION_FAILED),
        }
    }

    /// Segue as paginas do Drive ate a resposta que traz o arquivo. Devolve tambem o nome visto na confirmacao.
    fn resolve_file(&self, id: &str) -> Result<(Response, String, Option<String>), Error> {
        let mut url = format!("{}/uc?id={id}", self.host);
        // Caminho rapido: o servidor de download, sem redirecionamento nem pagina de confirmacao.
        // Documentos do Google e erros nao passam por aqui: seguem o caminho completo abaixo.
        let direct = format!("{}/download?id={id}&export=download&confirm=t", self.direct_host);
        if let Ok(response) = self.get(&direct, 0) {
            let is_html = header_text(&response, "content-type").is_some_and(|t| t.starts_with("text/html"));
            if response.status().as_u16() < 400 && !is_html && response.headers().contains_key("content-disposition") {
                return Ok((response, direct, None));
            }
        }
        let mut page_name = None;
        for _ in 0..4 {
            let mut response = self.get_checked(&url)?;
            let is_html = header_text(&response, "content-type").is_some_and(|t| t.starts_with("text/html"));
            if !is_html && response.headers().contains_key("content-disposition") {
                return Ok((response, url, page_name));
            }
            let final_url = response.get_uri().to_string();
            let export = [("/document/", "document", "docx"), ("/spreadsheets/", "spreadsheets", "xlsx"), ("/presentation/", "presentation", "pptx")]
                .iter()
                .find(|(path, ..)| final_url.contains(path) && !final_url.contains("/export"))
                .map(|(_, kind, format)| format!("{}/{kind}/d/{id}/export?format={format}", self.docs_host));
            if let Some(next) = export {
                url = next;
                continue;
            }
            let html = response.body_mut().read_to_string().unwrap_or_default();
            page_name = name_from_confirmation(&html).or(page_name);
            match confirmation_url(&html) {
                Some(next) => url = next,
                None => return msg(NO_ACCESS),
            }
        }
        msg(NO_ACCESS)
    }

    /// Baixa um arquivo para `dest`. Devolve None se ele ja existia (nao sobrescreve).
    fn download_file(
        &self,
        id: &str,
        listed_name: Option<&str>,
        dest: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(&str, u64, Option<u64>),
    ) -> Result<Option<std::path::PathBuf>, Error> {
        let (first, url, page_name) = self.resolve_file(id)?;
        let name = header_text(&first, "content-disposition")
            .and_then(|h| filename_from_disposition(&h))
            .or(page_name)
            .or_else(|| listed_name.map(clean_segment).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| id.to_string());
        fs::create_dir_all(dest)?;
        let final_path = dest.join(&name);
        if final_path.exists() {
            return Ok(None);
        }
        let part_path = dest.join(format!("{name}.part"));
        let existing = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

        let (response, start) = if existing == 0 {
            (first, 0)
        } else {
            drop(first);
            match self.get(&url, existing) {
                Ok(r) if r.status().as_u16() == 206 => (r, existing),
                Ok(r) if r.status().as_u16() == 416 => {
                    // o parcial ja tem o arquivo inteiro
                    drop(r);
                    fs::rename(&part_path, &final_path)?;
                    return Ok(Some(final_path));
                }
                Ok(r) if r.status().as_u16() < 400 => (r, 0), // o servidor ignorou o Range: recomeca
                Ok(_) => return msg(NO_ACCESS),
                Err(_) => return msg(CONNECTION_FAILED),
            }
        };

        let total = header_text(&response, "content-length").and_then(|v| v.trim().parse::<u64>().ok()).map(|n| n + start);
        let mut file = OpenOptions::new().create(true).write(true).append(start > 0).truncate(start == 0).open(&part_path)?;
        // A leitura roda em outra thread: cancelar e conexao travada nao dependem da rede responder.
        let (sender, receiver) = mpsc::sync_channel::<std::io::Result<Vec<u8>>>(16);
        let mut reader = response.into_body().into_reader();
        std::thread::spawn(move || loop {
            let mut buffer = vec![0u8; 64 * 1024];
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    buffer.truncate(read);
                    if sender.send(Ok(buffer)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        });
        let mut done = start;
        let mut last_data = Instant::now();
        on_bytes(&name, done, total);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(chunk)) => {
                    file.write_all(&chunk)?;
                    done += chunk.len() as u64;
                    last_data = Instant::now();
                    on_bytes(&name, done, total);
                }
                Ok(Err(_)) => return msg(CONNECTION_LOST),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if last_data.elapsed() >= self.stall_limit {
                        return msg(CONNECTION_LOST);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        file.flush()?;
        drop(file);
        if total.is_some_and(|t| t != done) {
            return msg("err.incomplete");
        }
        fs::rename(&part_path, &final_path)?;
        Ok(Some(final_path))
    }

    /// Lista os arquivos de uma pasta e subpastas: (id, subpasta relativa).
    fn list_folder(&self, id: &str, rel: &Path, seen: &mut HashSet<String>, out: &mut Vec<ListedFile>) -> Result<(), Error> {
        if !seen.insert(id.to_string()) {
            return Ok(());
        }
        let url = format!("{}/embeddedfolderview?id={id}", self.host);
        let html = self.get_checked(&url)?.body_mut().read_to_string().map_err(|_| Error::Message(NO_ACCESS.into()))?;
        for child in parse_folder_view(&html)? {
            match child {
                Child::File(file_id, name) => out.push((file_id, Some(name), rel.to_path_buf())),
                Child::Folder(folder_id, name) => {
                    let clean = clean_segment(&name);
                    let segment = if clean.is_empty() { folder_id.clone() } else { clean };
                    self.list_folder(&folder_id, &rel.join(segment), seen, out)?;
                }
            }
        }
        Ok(())
    }

    /// Baixa um link (arquivo ou pasta). Numa pasta, um arquivo com erro nao para os outros.
    pub fn download_link(&self, link: &Link, dest: &Path, cancel: &AtomicBool, on_progress: &mut dyn FnMut(Progress)) -> Result<Outcome, Error> {
        let Some(id) = extract_id(&link.url) else {
            return msg("err.noId");
        };
        let mut files = Vec::new();
        if link.kind == "folder" {
            self.list_folder(&id, Path::new(""), &mut HashSet::new(), &mut files)?;
            if files.is_empty() {
                return msg("err.emptyFolder");
            }
        } else {
            files.push((id, None, Default::default()));
        }

        let files_total = files.len();
        let mut outcome = Outcome { downloaded: 0, skipped: 0, files: Vec::new() };
        let mut failed: Vec<String> = Vec::new();
        for (index, (file_id, listed_name, rel)) in files.iter().enumerate() {
            let mut on_bytes = |file: &str, done: u64, total: Option<u64>| {
                on_progress(Progress { file: file.to_string(), done, total, files_done: index, files_total });
            };
            match self.download_file(file_id, listed_name.as_deref(), &dest.join(rel), cancel, &mut on_bytes) {
                Ok(Some(path)) => {
                    outcome.downloaded += 1;
                    outcome.files.push(path);
                }
                Ok(None) => outcome.skipped += 1,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(Error::Message(m)) => failed.push(m),
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
        }
        match failed.len() {
            0 => Ok(outcome),
            1 if files_total == 1 => Err(Error::Message(failed.remove(0))),
            n => Err(Error::Message(crate::i18n::code("err.someFiles", &[&n.to_string(), &files_total.to_string(), &failed[0]]))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    const ID_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const ID_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    const ID_SUB: &str = "SSSSSSSSSSSSSSSSSSSSSSSSSSSSSS";

    #[derive(Clone)]
    struct Route {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        // Declara um Content-Length maior que o corpo, simulando conexao que cai.
        declared: Option<usize>,
        ranges: bool,
        /// Cabecalho em bytes crus, como o Drive manda nomes acentuados.
        raw_header: Vec<u8>,
    }

    fn file_route(name: &str, body: &[u8]) -> Route {
        Route {
            status: 200,
            headers: vec![("Content-Disposition".into(), format!("attachment; filename=\"{name}\""))],
            body: body.to_vec(),
            declared: None,
            ranges: true,
            raw_header: Vec::new(),
        }
    }

    fn html_route(body: &str) -> Route {
        Route {
            status: 200,
            headers: vec![("Content-Type".into(), "text/html".into())],
            body: body.as_bytes().to_vec(),
            declared: None,
            ranges: false,
            raw_header: Vec::new(),
        }
    }

    type Routes = Arc<Mutex<HashMap<String, Route>>>;

    /// Servidor HTTP local minimo: nenhum teste acessa a rede.
    fn serve(routes: HashMap<String, Route>) -> (String, Routes, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let routes: Routes = Arc::new(Mutex::new(routes));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (shared, seen) = (routes.clone(), log.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut range: Option<usize> = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse().ok();
                    }
                }
                seen.lock().unwrap().push(format!("{path} {range:?}"));
                let route = shared.lock().unwrap().get(&path).cloned();
                let Some(route) = route else {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let (status, body) = match range {
                    Some(from) if route.ranges && from >= route.body.len() => (416, Vec::new()),
                    Some(from) if route.ranges => (206, route.body[from..].to_vec()),
                    _ => (route.status, route.body.clone()),
                };
                let length = route.declared.unwrap_or(body.len());
                let mut head = format!("HTTP/1.1 {status} X\r\nContent-Length: {length}\r\nConnection: close\r\n").into_bytes();
                for (k, v) in &route.headers {
                    head.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
                }
                head.extend_from_slice(&route.raw_header);
                head.extend_from_slice(b"\r\n");
                let _ = stream.write_all(&head);
                let _ = stream.write_all(&body);
            }
        });
        (address, routes, log)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("frey-drive-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(client: &Client, url: &str, kind: &'static str, dest: &Path) -> Result<Outcome, Error> {
        let link = Link { url: url.into(), kind };
        client.download_link(&link, dest, &AtomicBool::new(false), &mut |_| {})
    }

    fn file_url(id: &str) -> String {
        format!("https://drive.google.com/file/d/{id}/view?usp=sharing")
    }

    #[test]
    fn extrai_links_sem_repetir_e_sem_outros_sites() {
        let links = parse_links(&format!(
            "veja {} e\nhttps://drive.google.com/drive/folders/BBB?usp=sharing\n{}\nhttps://example.com/x\n{ID_A}",
            file_url("AAA"),
            file_url("AAA")
        ));
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].kind, "file");
        assert_eq!(links[1].kind, "folder");
        assert_eq!(links[2].url, ID_A);
    }

    #[test]
    fn reconhece_links_do_youtube() {
        let links = parse_links("https://www.youtube.com/watch?v=abc&t=3 https://youtu.be/xyz\nhttps://music.youtube.com/watch?v=m\nhttps://www.youtube.com/shorts/s https://evil-youtube.com/watch?v=n");
        let urls: Vec<_> = links.iter().map(|l| (l.url.as_str(), l.kind)).collect();
        assert_eq!(urls, vec![
            ("https://www.youtube.com/watch?v=abc&t=3", "youtube"),
            ("https://youtu.be/xyz", "youtube"),
            ("https://music.youtube.com/watch?v=m", "youtube"),
            ("https://www.youtube.com/shorts/s", "youtube"),
        ]);
    }

    #[test]
    fn extrai_id_de_todos_os_formatos() {
        assert_eq!(extract_id("https://drive.google.com/file/d/abc_-123/view").unwrap(), "abc_-123");
        assert_eq!(extract_id("https://drive.google.com/drive/folders/xyz?usp=sharing").unwrap(), "xyz");
        assert_eq!(extract_id("https://drive.google.com/open?id=qqq999").unwrap(), "qqq999");
        assert_eq!(extract_id("https://docs.google.com/document/d/doc123/edit").unwrap(), "doc123");
        assert_eq!(extract_id(ID_A).unwrap(), ID_A);
        assert_eq!(extract_id("https://drive.google.com/"), None);
    }

    #[test]
    fn nome_do_arquivo_com_acento_e_sem_caminho() {
        assert_eq!(filename_from_disposition("attachment; filename=\"a.mp4\"; filename*=UTF-8''Ca%C3%A7%C3%A3o%20final.mp4").unwrap(), "Cação final.mp4");
        assert_eq!(filename_from_disposition("attachment; filename=\"b.mov\"").unwrap(), "b.mov");
        assert_eq!(filename_from_disposition("attachment; filename=\"..\\\\evil.exe\"").unwrap(), ".._evil.exe".replace("_", "__"));
        assert_eq!(filename_from_disposition("inline"), None);
        // Percent malformado ou seguido de acento nao pode derrubar o app.
        assert_eq!(filename_from_disposition("attachment; filename*=UTF-8''%é%ZZ%4.mp4").unwrap(), "%é%ZZ%4.mp4");
    }

    #[test]
    fn le_a_pagina_de_pasta() {
        let html = format!(
            "<html><head><title>Pasta</title></head><body>
             <a href=\"https://drive.google.com/file/d/{ID_A}/view?usp=drive_web\" target=\"_blank\"><div>a.mp4</div></a>
             <a href=\"https://docs.google.com/document/d/{ID_B}/edit?usp=drive_web\"><div>Doc</div></a>
             <a href=\"https://drive.google.com/drive/folders/{ID_SUB}\"><div>Sub &amp; Cia</div></a>
             <a href=\"https://accounts.google.com/x\">login</a></body></html>"
        );
        let children = parse_folder_view(&html).unwrap();
        assert_eq!(children, vec![Child::File(ID_A.into(), "a.mp4".into()), Child::File(ID_B.into(), "Doc".into()), Child::Folder(ID_SUB.into(), "Sub & Cia".into())]);
        assert!(parse_folder_view("<html></html>").is_err());
    }

    #[test]
    fn acesso_direto_dispensa_a_pagina_de_confirmacao() {
        let path = format!("/download?id={ID_A}&export=download&confirm=t");
        let (host, _, log) = serve(HashMap::from([(path, file_route("grande.mov", b"direto"))]));
        let dest = temp_dir("fastpath");
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("grande.mov")).unwrap(), b"direto");
        assert!(!log.lock().unwrap().iter().any(|l| l.starts_with("/uc")), "nao passa pelo caminho antigo");
    }

    #[test]
    fn acesso_direto_com_pagina_html_cai_no_caminho_completo() {
        let direct = format!("/download?id={ID_A}&export=download&confirm=t");
        let (host, ..) = serve(HashMap::from([
            (direct, html_route("<html>sem acesso</html>")),
            (format!("/uc?id={ID_A}"), file_route("a.mp4", b"pelo caminho antigo")),
        ]));
        let dest = temp_dir("fallback");
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"pelo caminho antigo");
    }

    #[test]
    fn acesso_direto_retoma_o_parcial() {
        let direct = format!("/download?id={ID_A}&export=download&confirm=t");
        let (host, _, log) = serve(HashMap::from([(direct, file_route("a.mp4", b"0123456789"))]));
        let dest = temp_dir("fastresume");
        fs::write(dest.join("a.mp4.part"), b"01234").unwrap();
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"0123456789");
        assert!(log.lock().unwrap().iter().any(|l| l.ends_with("Some(5)")));
    }

    #[test]
    fn baixa_arquivo_direto() {
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), file_route("a.mp4", b"conteudo"))]));
        let dest = temp_dir("direct");
        let outcome = run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!((outcome.downloaded, outcome.skipped), (1, 0));
        assert_eq!(outcome.files, vec![dest.join("a.mp4")]);
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"conteudo");
        assert!(!dest.join("a.mp4.part").exists());
    }

    #[test]
    fn nome_com_acento_em_bytes_crus_nao_vira_falha() {
        // Regressao: arquivo com acento falhava como "Sem acesso".
        for (raw, expected) in [("Cação.pdf".as_bytes().to_vec(), "Cação.pdf"), (vec![0x41, 0xE7, 0xE3, 0x6F, 0x2E, 0x70, 0x64, 0x66], "Açãо.pdf".replace('о', "o").leak() as &str)] {
            let mut route = file_route("x", b"ok");
            route.headers.clear();
            route.raw_header = [b"Content-Disposition: attachment; filename=\"".as_slice(), &raw, b"\"\r\n"].concat();
            let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), route)]));
            let dest = temp_dir(&format!("raw{}", raw.len()));
            run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
            assert_eq!(fs::read(dest.join(expected)).unwrap(), b"ok");
        }
    }

    #[test]
    fn arquivo_grande_sem_nome_na_resposta_usa_o_da_confirmacao() {
        // O Drive responde Content-Disposition: inline depois da confirmacao.
        let (host, routes, _) = serve(HashMap::new());
        let page = format!(
            "<html><span class=\"uc-name-size\"><a href=\"/open?id={ID_A}\">Filme &amp; Cia ção.mp4</a> (2.5G)</span>
             <form id=\"download-form\" action=\"{host}/download\" method=\"get\">
             <input type=\"hidden\" name=\"id\" value=\"{ID_A}\"><input type=\"hidden\" name=\"confirm\" value=\"t\"></form></html>"
        );
        routes.lock().unwrap().insert(format!("/uc?id={ID_A}"), html_route(&page));
        let mut inline = file_route("x", b"video");
        inline.headers = vec![("Content-Disposition".into(), "inline".into()), ("Content-Type".into(), "video/mp4".into())];
        routes.lock().unwrap().insert(format!("/download?id={ID_A}&confirm=t"), inline);
        let dest = temp_dir("inline");
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("Filme & Cia ção.mp4")).unwrap(), b"video");
    }

    #[test]
    fn sem_nome_em_lugar_nenhum_usa_o_da_listagem_da_pasta() {
        let view = format!("<html><title>P</title><a href=\"https://drive.google.com/file/d/{ID_A}/view\"><div>Listado ção.mov</div></a></html>");
        let mut inline = file_route("x", b"video");
        inline.headers = vec![("Content-Disposition".into(), "inline".into())];
        let (host, ..) = serve(HashMap::from([
            (format!("/embeddedfolderview?id={ID_SUB}"), html_route(&view)),
            (format!("/uc?id={ID_A}"), inline),
        ]));
        let dest = temp_dir("listed");
        let url = format!("https://drive.google.com/drive/folders/{ID_SUB}");
        run(&Client::with_hosts(&host, &host), &url, "folder", &dest).unwrap();
        assert_eq!(fs::read(dest.join("Listado ção.mov")).unwrap(), b"video");
    }

    #[test]
    fn conexao_travada_vira_erro_em_vez_de_congelar() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Disposition: attachment; filename=\"a.mp4\"\r\n\r\nparcial");
            std::thread::sleep(Duration::from_secs(20)); // fica calado sem fechar
        });
        let mut client = Client::with_hosts(&host, &host);
        client.stall_limit = Duration::from_millis(600);
        let dest = temp_dir("stall");
        let started = Instant::now();
        let result = run(&client, &file_url(ID_A), "file", &dest);
        assert_eq!(result, Err(Error::Message(CONNECTION_LOST.into())));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(fs::read(dest.join("a.mp4.part")).unwrap(), b"parcial", "o parcial fica para retomar");
    }

    #[test]
    fn passa_pela_confirmacao_de_arquivo_grande() {
        let (host, routes, _) = serve(HashMap::new());
        let page = format!(
            "<html><form id=\"download-form\" action=\"{host}/download\" method=\"get\">
             <input type=\"hidden\" name=\"id\" value=\"{ID_A}\"><input type=\"hidden\" name=\"confirm\" value=\"t\">
             <input type=\"submit\" value=\"Baixar\"></form></html>"
        );
        routes.lock().unwrap().insert(format!("/uc?id={ID_A}"), html_route(&page));
        routes.lock().unwrap().insert(format!("/download?id={ID_A}&confirm=t"), file_route("grande.mov", b"grande"));
        let dest = temp_dir("confirm");
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("grande.mov")).unwrap(), b"grande");
    }

    #[test]
    fn pagina_sem_arquivo_vira_erro_de_acesso() {
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), html_route("<html>Too many users</html>"))]));
        let dest = temp_dir("noaccess");
        let client = Client::with_hosts(&host, &host);
        assert_eq!(run(&client, &file_url(ID_A), "file", &dest), Err(Error::Message(NO_ACCESS.into())));
        assert_eq!(run(&client, &file_url(ID_B), "file", &dest), Err(Error::Message(NO_ACCESS.into())), "404");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn nao_sobrescreve_arquivo_existente() {
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), file_route("a.mp4", b"novo"))]));
        let dest = temp_dir("skip");
        fs::write(dest.join("a.mp4"), b"original").unwrap();
        let outcome = run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!((outcome.downloaded, outcome.skipped), (0, 1));
        assert!(outcome.files.is_empty());
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"original");
    }

    #[test]
    fn conexao_que_cai_mantem_parcial_e_retoma() {
        let mut route = file_route("a.mp4", b"0123456789");
        route.declared = Some(20); // promete 20, entrega 10 e fecha
        route.ranges = false;
        let path = format!("/uc?id={ID_A}");
        let (host, routes, log) = serve(HashMap::from([(path.clone(), route)]));
        let dest = temp_dir("resume");
        let client = Client::with_hosts(&host, &host);
        assert!(run(&client, &file_url(ID_A), "file", &dest).is_err());
        assert_eq!(fs::read(dest.join("a.mp4.part")).unwrap(), b"0123456789");
        assert!(!dest.join("a.mp4").exists());

        routes.lock().unwrap().insert(path.clone(), file_route("a.mp4", b"0123456789ABCDEFGHIJ"));
        run(&client, &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"0123456789ABCDEFGHIJ");
        assert!(!dest.join("a.mp4.part").exists());
        assert!(log.lock().unwrap().iter().any(|l| l.ends_with("Some(10)")), "pediu so o que faltava");
    }

    #[test]
    fn servidor_que_ignora_range_recomeca_do_zero() {
        let mut route = file_route("a.mp4", b"completo");
        route.ranges = false;
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), route)]));
        let dest = temp_dir("norange");
        fs::write(dest.join("a.mp4.part"), b"lixo antigo maior").unwrap();
        run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"completo");
    }

    #[test]
    fn parcial_ja_completo_so_e_renomeado() {
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), file_route("a.mp4", b"abc"))]));
        let dest = temp_dir("complete");
        fs::write(dest.join("a.mp4.part"), b"abc").unwrap();
        let outcome = run(&Client::with_hosts(&host, &host), &file_url(ID_A), "file", &dest).unwrap();
        assert_eq!(outcome.downloaded, 1);
        assert_eq!(fs::read(dest.join("a.mp4")).unwrap(), b"abc");
    }

    #[test]
    fn pasta_com_subpasta_e_falha_que_nao_para_os_outros() {
        let view = format!("<html><title>P</title><a href=\"https://drive.google.com/file/d/{ID_A}/view\">a</a><a href=\"https://drive.google.com/file/d/{ID_B}/view\">b</a><a href=\"https://drive.google.com/drive/folders/{ID_SUB}\"><div>Sub</div></a></html>");
        let sub = format!("<html><title>S</title><a href=\"https://drive.google.com/file/d/{ID_A}/view\">a</a></html>");
        let (host, ..) = serve(HashMap::from([
            (format!("/embeddedfolderview?id={ID_SUB}1"), html_route("x")),
            (format!("/embeddedfolderview?id=ROOTROOTROOTROOTROOTROOTROOT"), html_route(&view)),
            (format!("/embeddedfolderview?id={ID_SUB}"), html_route(&sub)),
            (format!("/uc?id={ID_A}"), file_route("a.mp4", b"a")),
            // ID_B sem rota: 404
        ]));
        let dest = temp_dir("folder");
        let result = run(&Client::with_hosts(&host, &host), "https://drive.google.com/drive/folders/ROOTROOTROOTROOTROOTROOTROOT", "folder", &dest);
        assert_eq!(result, Err(Error::Message(crate::i18n::code("err.someFiles", &["1", "3", NO_ACCESS]))));
        assert!(dest.join("a.mp4").exists(), "conteudo vai direto para o destino");
        assert!(dest.join("Sub").join("a.mp4").exists(), "subpasta preservada");
    }

    #[test]
    fn pasta_vazia_ou_sem_acesso_e_falha() {
        let (host, ..) = serve(HashMap::from([(format!("/embeddedfolderview?id={ID_SUB}"), html_route("<html><title>Entrar</title></html>"))]));
        let dest = temp_dir("empty");
        let url = format!("https://drive.google.com/drive/folders/{ID_SUB}");
        let result = run(&Client::with_hosts(&host, &host), &url, "folder", &dest);
        assert_eq!(result, Err(Error::Message("err.emptyFolder".into())));
    }

    #[test]
    fn cancelar_para_e_mantem_o_parcial() {
        let (host, ..) = serve(HashMap::from([(format!("/uc?id={ID_A}"), file_route("a.mp4", &vec![7u8; 400_000]))]));
        let dest = temp_dir("cancel");
        let cancel = AtomicBool::new(false);
        let link = Link { url: file_url(ID_A), kind: "file" };
        let result = Client::with_hosts(&host, &host).download_link(&link, &dest, &cancel, &mut |p| {
            if p.done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert_eq!(result, Err(Error::Cancelled));
        assert!(!dest.join("a.mp4").exists());
        assert!(dest.join("a.mp4.part").exists());
    }

    #[test]
    fn documento_google_usa_exportacao() {
        // Sem Content-Disposition e com /document/ na URL: troca para a exportacao.
        let (host, routes, _) = serve(HashMap::new());
        let id = ID_B;
        routes.lock().unwrap().insert(format!("/document/x/{id}/edit"), html_route("<html>doc</html>"));
        let mut redirect = html_route("");
        redirect.status = 302;
        redirect.headers = vec![("Location".into(), format!("{host}/document/x/{id}/edit"))];
        routes.lock().unwrap().insert(format!("/uc?id={id}"), redirect);
        routes.lock().unwrap().insert(format!("/document/d/{id}/export?format=docx"), file_route("Doc.docx", b"docx"));
        let dest = temp_dir("doc");
        run(&Client::with_hosts(&host, &host), &format!("https://docs.google.com/document/d/{id}/edit"), "file", &dest).unwrap();
        assert_eq!(fs::read(dest.join("Doc.docx")).unwrap(), b"docx");
    }
}
