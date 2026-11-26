// Frey Project: painel residente chamado por atalho global (como o Listary ou a barra do Finder).
// Fica na bandeja, cria a estrutura do projeto e baixa os links do Drive. Nao fala com o Premiere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod convert;
mod drive;
mod history;
mod i18n;
mod models;
mod outside_click;
mod places;
mod platform;
mod youtube;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

#[cfg(not(target_os = "macos"))]
const DEFAULT_HOTKEY: &str = "ctrl+alt+p";
#[cfg(target_os = "macos")]
const DEFAULT_HOTKEY: &str = "cmd+alt+p";
const PANEL_WIDTH: f64 = 640.0;
const TRAY_ID: &str = "main";
const HIDDEN_ARG: &str = "--hidden";
#[cfg(windows)]
const JUST_OPENED: std::time::Duration = std::time::Duration::from_millis(1500);

#[derive(Default, Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    modelo: String,
    #[serde(default)]
    destino: String,
    #[serde(default)]
    atalho: String,
    #[serde(default)]
    idioma: String,
    #[serde(default)]
    so_baixar: bool,
    #[serde(default)]
    ignorar_link_copiado: bool,
    /// Login do YouTube: "" desligado, "firefox", "brave", "safari" e "chrome" (Mac) ou "file".
    #[serde(default)]
    youtube_login: String,
    #[serde(default)]
    youtube_cookies: String,
    /// O usuario marcou que entende o risco para a conta. Sem isso o login fica desligado.
    #[serde(default)]
    youtube_risco_aceito: bool,
    /// Onde o login automatico achou a conta de cada site: "youtube" -> "firefox", "instagram" -> "frey"...
    #[serde(default)]
    login_encontrado: std::collections::BTreeMap<String, String>,
    /// Ultimos destinos usados, do mais novo para o mais antigo.
    #[serde(default)]
    historico: Vec<String>,
}

/// Arquivo com o login feito pela janela do proprio Frey ("Entrar pelo Frey").
fn frey_login_path(app: &AppHandle) -> PathBuf {
    config_dir(app).unwrap_or_default().join("login-frey.txt")
}

/// Contas para tentar quando o site pede login, na ordem. Sem o aceite de risco, nenhuma.
/// "auto": login do Frey, depois o navegador que ja funcionou, depois os outros navegadores instalados.
fn logins_for(app: &AppHandle, url: &str) -> Vec<youtube::Login> {
    let settings = app.state::<Shared>().settings.lock().unwrap().clone();
    let Some(site) = youtube::site(url) else { return Vec::new() };
    if !settings.youtube_risco_aceito {
        return Vec::new();
    }
    if settings.youtube_login != "auto" {
        return youtube::Login::from_settings(&settings.youtube_login, &settings.youtube_cookies).into_iter().collect();
    }
    let frey = frey_login_path(app);
    let mut list = Vec::new();
    if login_state_of(&frey).contains(&site) {
        list.push(youtube::Login::File(frey));
    }
    let remembered = settings.login_encontrado.get(site).cloned().unwrap_or_default();
    let mut browsers: Vec<&str> = youtube::installed_browsers();
    if let Some(at) = browsers.iter().position(|b| *b == remembered) {
        let found = browsers.remove(at);
        browsers.insert(0, found);
    }
    list.extend(browsers.into_iter().map(|b| youtube::Login::Browser(b.to_string())));
    list
}

/// Guarda onde a conta foi achada, para a proxima vez comecar por ali.
fn remember_login(app: &AppHandle, url: &str, login: &youtube::Login) {
    let Some(site) = youtube::site(url) else { return };
    let source = match login {
        youtube::Login::File(path) if *path == frey_login_path(app) => "frey".to_string(),
        other => other.source(),
    };
    let state = app.state::<Shared>();
    let mut settings = state.settings.lock().unwrap();
    if settings.login_encontrado.get(site) != Some(&source) {
        settings.login_encontrado.insert(site.to_string(), source);
        save_settings(app, &settings);
    }
}

fn login_label(app: &AppHandle, login: &youtube::Login) -> String {
    match login {
        youtube::Login::File(path) if *path == frey_login_path(app) => "frey".into(),
        other => other.source(),
    }
}

#[derive(Default)]
struct Shared {
    settings: Mutex<Settings>,
    hotkey_error: Mutex<Option<String>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// (modelo|nome|destino, pasta do projeto): tentar de novo nao pergunta outra vez.
    last_project: Mutex<Option<(String, PathBuf)>>,
    last_converted: Mutex<Option<PathBuf>>,
    /// Lotes de download esperando o atual terminar (colar links enquanto baixa entra na fila).
    queue: Mutex<std::collections::VecDeque<Batch>>,
    /// O trabalho em andamento e de download (e nao de conversao): so ele aceita fila.
    downloading: AtomicBool,
    /// Item baixando agora e o sinal para parar so ele.
    item_cancel: Mutex<Option<(u32, Arc<AtomicBool>)>>,
    /// Itens na fila que o usuario tirou antes de comecar.
    skip: Mutex<std::collections::HashSet<u32>>,
    history: Mutex<Option<Vec<history::Entry>>>,
    gpu: Mutex<Option<Option<String>>>,
    dialog_open: AtomicBool,
    panel_shown_at: Mutex<Option<Instant>>,
    clipboard: clipboard::Watch,
}

fn panel(app: &AppHandle) -> WebviewWindow {
    app.get_webview_window("panel").expect("janela do painel")
}

/// Pasta das configuracoes. A variavel existe para os testes nunca tocarem nos dados do usuario.
fn config_dir(app: &AppHandle) -> Option<PathBuf> {
    match std::env::var_os("FREY_PROJECT_CONFIG_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => app.path().app_config_dir().ok(),
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    config_dir(app).map(|d| d.join("settings.json"))
}

fn config_path(app: &AppHandle) -> PathBuf {
    config_dir(app).unwrap_or_default().join("modelos.json")
}

fn language(app: &AppHandle) -> String {
    app.state::<Shared>().settings.lock().unwrap().idioma.clone()
}

fn save_settings(app: &AppHandle, settings: &Settings) {
    if let (Some(path), Ok(json)) = (settings_path(app), serde_json::to_string_pretty(settings)) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, json); // perder a lembranca nao pode travar o trabalho
    }
}

#[cfg(windows)]
/// Ponto na borda direita do monitor mais a direita: nenhuma tela mostra a janela a partir dali.
/// O Windows so aceita criar a janela com a origem dentro de um monitor, por isso nao vai mais longe.
fn beyond_the_screens(app: &AppHandle) -> Option<(f64, f64)> {
    let monitors = app.available_monitors().ok()?;
    let right = monitors.iter().max_by_key(|m| m.position().x + m.size().width as i32)?;
    let scale = right.scale_factor();
    let x = (right.position().x + right.size().width as i32) as f64 / scale - 1.0;
    Some((x, right.position().y as f64 / scale))
}

/// Abre no monitor do cursor, no terco superior, como as barras de acesso rapido.
fn show_panel(app: &AppHandle) {
    let window = panel(app);
    let mut placed = false;
    if let (Ok(cursor), Ok(monitors), Ok(size)) = (app.cursor_position(), app.available_monitors(), window.outer_size()) {
        let monitor = monitors.iter().find(|m| {
            let (p, s) = (m.position(), m.size());
            cursor.x >= p.x as f64 && cursor.x < p.x as f64 + s.width as f64 && cursor.y >= p.y as f64 && cursor.y < p.y as f64 + s.height as f64
        });
        if let Some(m) = monitor {
            let (p, s) = (m.position(), m.size());
            let x = p.x + (s.width as i32 - size.width as i32) / 2;
            let y = p.y + (s.height as f64 * 0.2) as i32;
            placed = window.set_position(PhysicalPosition::new(x, y)).is_ok();
        }
    }
    // A janela nasce fora da tela: sem o cursor em um monitor conhecido, nao pode abrir la.
    if !placed {
        let _ = window.center();
    }
    let state = app.state::<Shared>();
    *state.panel_shown_at.lock().unwrap() = Some(Instant::now());
    let _ = window.show();
    let _ = window.set_focus();
    #[cfg(windows)]
    take_foreground(window.hwnd().ok().map(|h| h.0 as *mut std::ffi::c_void));
    let _ = window.emit("panel-shown", ());
}

fn toggle_panel(app: &AppHandle) {
    let window = panel(app);
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        show_panel(app);
    }
}

/// Abre a pasta no Explorer; se ela ja estiver aberta, traz essa janela em vez de abrir outra.
/// Roda em outra thread porque perguntar ao Windows leva alguns décimos de segundo.
#[cfg(windows)]
fn open_folder(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || match find_open_folder(&path) {
        Some(window) => bring_to_front(window),
        None => {
            let _ = std::process::Command::new("explorer.exe").arg(&path).spawn();
        }
    });
}

/// No Mac o Finder ja traz a janela aberta na pasta em vez de abrir outra.
#[cfg(not(windows))]
fn open_folder(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

#[cfg(windows)]
const FIND_FOLDER_SCRIPT: &str = "$want=[IO.Path]::GetFullPath($env:FREY_OPEN_PATH).TrimEnd('\\');foreach($w in (New-Object -ComObject Shell.Application).Windows()){try{$p=$w.Document.Folder.Self.Path;if($p -and $p.TrimEnd('\\') -ieq $want){[int64]$w.HWND;break}}catch{}}";

/// Janela do Explorer ja aberta nesta pasta. Na duvida devolve None, e abre uma janela nova como antes.
#[cfg(windows)]
fn find_open_folder(path: &Path) -> Option<isize> {
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", FIND_FOLDER_SCRIPT])
        .env("FREY_OPEN_PATH", path)
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed().as_secs() < 5 => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    text.trim().parse::<isize>().ok().filter(|window| *window != 0)
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn IsIconic(window: *mut std::ffi::c_void) -> i32;
    fn ShowWindow(window: *mut std::ffi::c_void, command: i32) -> i32;
    fn SetForegroundWindow(window: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
fn bring_to_front(window: isize) {
    const SW_RESTORE: i32 = 9;
    let window = window as *mut std::ffi::c_void;
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
    }
}

// ---------------------------------------------------------------- comandos do painel

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Boot {
    models: Vec<String>,
    default_name: String,
    model: String,
    dest: String,
    hotkey: String,
    hotkey_error: Option<String>,
    config_error: Option<String>,
    running: bool,
    language: String,
    visible: bool,
    download_only: bool,
    detect_link: bool,
    /// "windows" ou "macos": o painel mostra os atalhos do sistema.
    platform: &'static str,
    youtube_login: String,
    youtube_cookies: String,
    youtube_risk: bool,
}

#[tauri::command]
fn boot(app: AppHandle, state: State<Shared>) -> Boot {
    let settings = state.settings.lock().unwrap().clone();
    let config = models::load_config(&config_path(&app));
    let language = settings.idioma.clone();
    let visible = panel(&app).is_visible().unwrap_or(false);
    let download_only = settings.so_baixar;
    let detect_link = !settings.ignorar_link_copiado;
    let (yt_login, yt_cookies, yt_risk) = (settings.youtube_login.clone(), settings.youtube_cookies.clone(), settings.youtube_risco_aceito);
    let hotkey = if settings.atalho.is_empty() { DEFAULT_HOTKEY.to_string() } else { settings.atalho.clone() };
    let running = state.cancel.lock().unwrap().is_some();
    let hotkey_error = state.hotkey_error.lock().unwrap().clone();
    match config {
        Ok(cfg) => {
            let names: Vec<String> = cfg.templates.iter().map(|t| t.name.clone()).collect();
            let model = [settings.modelo.as_str(), models::DEFAULT_MODEL]
                .iter()
                .find(|m| names.iter().any(|n| n == *m))
                .map(|m| m.to_string())
                .unwrap_or_else(|| names[0].clone());
            let dest = if Path::new(&settings.destino).is_dir() { settings.destino } else { String::new() };
            Boot { models: names, default_name: cfg.default_name, model, dest, hotkey, hotkey_error, config_error: None, running, language, visible, download_only, detect_link, platform: std::env::consts::OS, youtube_login: yt_login, youtube_cookies: yt_cookies, youtube_risk: yt_risk }
        }
        Err(e) => Boot {
            models: Vec::new(),
            default_name: String::new(),
            model: String::new(),
            dest: String::new(),
            hotkey,
            hotkey_error,
            config_error: Some(e),
            platform: std::env::consts::OS,
            youtube_login: yt_login,
            youtube_cookies: yt_cookies,
            youtube_risk: yt_risk,
            running,
            language,
            visible,
            download_only,
            detect_link,
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    root: String,
    folders: Vec<String>,
    default_folder: String,
    needs_name: bool,
}

#[tauri::command]
fn preview(app: AppHandle, model: String, name: String) -> Result<Preview, String> {
    let config = models::load_config(&config_path(&app))?;
    let template = config.templates.iter().find(|t| t.name == model).ok_or("err.modelGone")?;
    let plan = models::build_plan(template, &name);
    let folders = models::target_folders(&plan);
    let default_folder = models::destination(template, &plan, &name);
    Ok(Preview { root: plan.root, folders, default_folder, needs_name: models::needs_name(template) })
}

#[tauri::command]
fn parse_links(text: String) -> Vec<drive::Link> {
    drive::parse_links(&text)
}

#[tauri::command(async)]
fn pick_folder(app: AppHandle, state: State<'_, Shared>, current: String) -> Option<String> {
    state.dialog_open.store(true, Ordering::SeqCst);
    let mut dialog = app.dialog().file().set_title(i18n::native(&language(&app), "dialogDest")).set_parent(&panel(&app));
    if Path::new(&current).is_dir() {
        dialog = dialog.set_directory(&current);
    }
    let picked = dialog.blocking_pick_folder();
    state.dialog_open.store(false, Ordering::SeqCst);
    let _ = panel(&app).set_focus();
    picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

#[tauri::command]
fn recent_paths(state: State<Shared>) -> Vec<places::Place> {
    places::recent(&state.settings.lock().unwrap().historico)
}

#[tauri::command]
fn forget_path(app: AppHandle, state: State<Shared>, path: String) -> Vec<places::Place> {
    let mut settings = state.settings.lock().unwrap();
    places::forget(&mut settings.historico, &path);
    save_settings(&app, &settings);
    places::recent(&settings.historico)
}

/// Subpastas da pasta escolhida no modo So baixar: cada link pode ir para uma delas.
#[tauri::command(async)]
fn list_subfolders(path: String) -> Vec<String> {
    if path.trim().is_empty() {
        return Vec::new();
    }
    places::subfolders(Path::new(path.trim())).into_iter().map(|p| p.name).collect()
}

#[derive(Deserialize)]
struct JobItem {
    id: u32,
    url: String,
    kind: String,
    folder: String,
    /// So para links baixados pelo yt-dlp.
    #[serde(default)]
    quality: String,
    /// Titulo e capa da previa, para o historico.
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumb: String,
    /// Trecho em segundos (inicio e fim); sem os dois, o video inteiro.
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    /// Baixar a legenda junto.
    #[serde(default)]
    subs: bool,
}

/// Um "Baixar" do painel: os links, a pasta base e o que fazer depois.
struct Batch {
    items: Vec<JobItem>,
    base: PathBuf,
    open_at_end: bool,
    convert_to: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    model: String,
    name: String,
    dest: String,
    items: Vec<JobItem>,
    confirm_existing: bool,
    /// Formato para converter depois de baixar; vazio mantem so o original.
    convert_to: String,
    /// Baixa direto na pasta escolhida, sem criar a estrutura do modelo.
    #[serde(default)]
    download_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartAnswer {
    status: &'static str, // needs_confirm | created | started | queued
    root: String,
    base: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum JobEvent {
    Start { id: u32 },
    Progress { id: u32, progress: drive::Progress },
    Converting { id: u32, file: String, percent: f64 },
    Ok { id: u32, downloaded: usize, skipped: usize, converted: usize, convert_error: Option<String> },
    Fail { id: u32, message: String },
    Cancelled { id: u32 },
    Done { failed: usize, ok: usize, cancelled: bool, base: String, more: bool },
}

#[tauri::command]
fn start_job(app: AppHandle, state: State<Shared>, request: StartRequest) -> Result<StartAnswer, String> {
    // Uma conversao em andamento ocupa o app; um download em andamento so recebe o novo lote na fila.
    if state.cancel.lock().unwrap().is_some() && !state.downloading.load(Ordering::SeqCst) {
        return Err("err.busy".into());
    }
    if request.download_only {
        return start_download_only(app, state, request);
    }
    let config = models::load_config(&config_path(&app))?;
    let template = config.templates.iter().find(|t| t.name == request.model).ok_or("err.modelGone")?;
    let name = request.name.trim();
    let dest = PathBuf::from(request.dest.trim());
    if !dest.is_dir() {
        return Err("err.destInvalid".into());
    }
    if models::needs_name(template) && models::clean_segment(name).is_empty() {
        return Err("err.nameNeeded".into());
    }

    if request.items.iter().any(|i| youtube::handles(&i.kind)) && convert::find_tool("yt-dlp").is_none() {
        return Err(youtube::MISSING.into());
    }
    if !request.convert_to.is_empty() && !convert::is_output_format(&request.convert_to) {
        return Err("err.formatInvalid".into());
    }
    if !request.convert_to.is_empty() && convert::find_tool("ffmpeg").is_none() {
        return Err(convert::FFMPEG_MISSING.into());
    }

    let plan = models::build_plan(template, name);
    let folders = models::target_folders(&plan);
    if let Some(bad) = request.items.iter().find(|i| !i.folder.is_empty() && !folders.contains(&i.folder)) {
        return Err(i18n::code("err.folderNotInModel", &[&bad.folder]));
    }

    let signature = format!("{}|{name}|{}", request.model, dest.display());
    let previous = state.last_project.lock().unwrap().clone().filter(|(s, _)| *s == signature);
    let base = match previous {
        Some((_, base)) => base,
        None => {
            let base = models::project_base(&plan, &dest);
            if !plan.root.is_empty() && base.exists() && !request.confirm_existing {
                return Ok(StartAnswer { status: "needs_confirm", root: plan.root, base: base.display().to_string() });
            }
            base
        }
    };
    let created = if base == models::project_base(&plan, &dest) {
        models::create_on_disk(&plan, &dest).map_err(|e| i18n::code("err.createFolders", &[&e.to_string()]))?
    } else {
        base
    };
    *state.last_project.lock().unwrap() = Some((signature, created.clone()));
    {
        let mut settings = state.settings.lock().unwrap();
        settings.modelo = request.model.clone();
        settings.destino = dest.display().to_string();
        places::remember(&mut settings.historico, &dest.display().to_string());
        save_settings(&app, &settings);
    }

    let answer_base = created.display().to_string();
    if request.items.is_empty() {
        if config.open_at_end {
            open_folder(&created);
        }
        return Ok(StartAnswer { status: "created", root: plan.root, base: answer_base });
    }

    let status = launch(&app, Batch { items: request.items, base: created, open_at_end: config.open_at_end, convert_to: request.convert_to });
    Ok(StartAnswer { status, root: plan.root, base: answer_base })
}

/// Modo "so baixar": nenhuma pasta e criada e os modelos nem sao consultados para a estrutura.
fn start_download_only(app: AppHandle, state: State<Shared>, request: StartRequest) -> Result<StartAnswer, String> {
    let dest = PathBuf::from(request.dest.trim());
    if !dest.is_dir() {
        return Err("err.destDownload".into());
    }
    if request.items.is_empty() {
        return Err("err.noLinks".into());
    }
    if request.items.iter().any(|i| youtube::handles(&i.kind)) && convert::find_tool("yt-dlp").is_none() {
        return Err(youtube::MISSING.into());
    }
    if !request.convert_to.is_empty() && !convert::is_output_format(&request.convert_to) {
        return Err("err.formatInvalid".into());
    }
    if !request.convert_to.is_empty() && convert::find_tool("ffmpeg").is_none() {
        return Err(convert::FFMPEG_MISSING.into());
    }
    *state.last_project.lock().unwrap() = Some((format!("so-baixar|{}", dest.display()), dest.clone()));
    {
        let mut settings = state.settings.lock().unwrap();
        settings.destino = dest.display().to_string();
        places::remember(&mut settings.historico, &dest.display().to_string());
        save_settings(&app, &settings);
    }
    // Modelos com erro nao impedem baixar: nesse caso vale abrir a pasta ao terminar.
    let open_at_end = models::load_config(&config_path(&app)).map(|c| c.open_at_end).unwrap_or(true);
    // Cada link pode ir para uma subpasta direta da pasta escolhida; nada e criado aqui.
    let mut items = Vec::with_capacity(request.items.len());
    for item in request.items {
        let folder = if item.folder.is_empty() {
            String::new()
        } else {
            match places::subfolder(&dest, &item.folder) {
                Some(_) => item.folder.clone(),
                None => return Err(i18n::code("err.subfolderGone", &[&item.folder])),
            }
        };
        items.push(JobItem { folder, ..item });
    }
    let status = launch(&app, Batch { items, base: dest.clone(), open_at_end, convert_to: request.convert_to });
    Ok(StartAnswer { status, root: String::new(), base: dest.display().to_string() })
}

/// GPU detectada uma vez por sessao (o teste leva alguns segundos).
fn cached_gpu(app: &AppHandle) -> Option<String> {
    let state = app.state::<Shared>();
    if let Some(known) = state.gpu.lock().unwrap().as_ref() {
        return known.clone();
    }
    let found = convert::find_tool("ffmpeg").and_then(|ffmpeg| convert::detect_gpu(&ffmpeg));
    *state.gpu.lock().unwrap() = Some(found.clone());
    found
}

/// Converte o que foi baixado. Arquivo que nao e midia ou ja esta no formato fica como veio.
fn convert_downloaded(app: &AppHandle, id: u32, files: &[PathBuf], ext: &str, cancel: &AtomicBool) -> (usize, Option<String>) {
    let (mut converted, mut first_error) = (0, None);
    for file in files {
        // Mesmo formato so e pulado se o video ja serve: AV1 dentro de MP4 ainda precisa virar H.264.
        let needs_h264 = convert::needs_h264(file, ext);
        if !convert::is_media(file) || (convert::extension(file).as_deref() == Some(ext) && !needs_h264) {
            continue;
        }
        let options = convert::Options {
            ext: ext.into(),
            mode: if needs_h264 { "encode".into() } else { "auto".into() },
            crf: if needs_h264 { 18 } else { 22 },
            gpu: cached_gpu(app),
            out_dir: None,
        };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut last_emit = Instant::now();
        let mut on_progress = |percent: f64, _: &str| {
            if last_emit.elapsed().as_millis() >= 150 {
                last_emit = Instant::now();
                let _ = app.emit("job", JobEvent::Converting { id, file: name.clone(), percent });
            }
        };
        match convert::convert_file(file, &options, cancel, &mut on_progress) {
            Ok(_) => converted += 1,
            Err(convert::Error::Cancelled) => break,
            Err(convert::Error::Message(message)) => first_error = first_error.or(Some(message)),
        }
    }
    (converted, first_error)
}

/// "4K para edicao": baixa o maior video ate 4K em pasta temporaria e entrega H.264 em MP4.
/// A pasta tem nome fixo por link: cancelar e tentar de novo continua de onde parou.
/// Se a conversao falhar, o arquivo baixado vai para o destino e o erro volta em `edit_error`.
fn download_for_editing(
    app: &AppHandle,
    id: u32,
    url: &str,
    target: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(drive::Progress),
    edit_error: &mut Option<String>,
) -> Result<drive::Outcome, drive::Error> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(url, &mut hasher);
    let temp = target.join(format!(".frey-4k-{:016x}", std::hash::Hasher::finish(&hasher)));
    let logins = logins_for(app, url);
    youtube::download_with(url, &temp, "edit4k", &youtube::Extra::default(), &logins, &mut |login| remember_login(app, url, login), cancel, on_progress)?;

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&temp)?.filter_map(Result::ok).map(|e| e.path()).filter(|p| convert::is_media(p)).collect();
    sources.sort();
    let mut outcome = drive::Outcome { downloaded: 0, skipped: 0, files: Vec::new() };
    for source in sources {
        outcome.downloaded += 1;
        if convert::extension(&source).as_deref() == Some("mp4") && !convert::needs_h264(&source, "mp4") {
            let final_path = convert::output_path(&source, "mp4", Some(target));
            std::fs::rename(&source, &final_path)?;
            outcome.files.push(final_path);
            continue;
        }
        let options = convert::Options { ext: "mp4".into(), mode: "encode".into(), crf: 18, gpu: cached_gpu(app), out_dir: Some(target.to_path_buf()) };
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut last_emit = Instant::now();
        let mut on_convert = |percent: f64, _: &str| {
            if last_emit.elapsed().as_millis() >= 150 {
                last_emit = Instant::now();
                let _ = app.emit("job", JobEvent::Converting { id, file: name.clone(), percent });
            }
        };
        match convert::convert_file(&source, &options, cancel, &mut on_convert) {
            Ok(output) => {
                let _ = std::fs::remove_file(&source);
                outcome.files.push(output);
            }
            Err(convert::Error::Cancelled) => return Err(drive::Error::Cancelled),
            Err(convert::Error::Message(message)) => {
                let extension = convert::extension(&source).unwrap_or_default();
                let kept = convert::output_path(&source, &extension, Some(target));
                std::fs::rename(&source, &kept)?;
                outcome.files.push(kept);
                *edit_error = Some(message);
            }
        }
    }
    let _ = std::fs::remove_dir(&temp); // so remove se estiver vazia
    Ok(outcome)
}

/// Comeca o lote agora, ou poe na fila se um download ja esta rodando. Devolve "started" ou "queued".
fn launch(app: &AppHandle, batch: Batch) -> &'static str {
    let state = app.state::<Shared>();
    let mut running = state.cancel.lock().unwrap();
    if running.is_some() {
        state.queue.lock().unwrap().push_back(batch);
        return "queued";
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *running = Some(cancel.clone());
    state.downloading.store(true, Ordering::SeqCst);
    let worker_app = app.clone();
    std::thread::spawn(move || run_queue(worker_app, batch, cancel));
    "started"
}

/// Baixa o lote e depois os que entraram na fila. Cancelar tudo cancela tambem os da fila.
fn run_queue(app: AppHandle, first: Batch, cancel: Arc<AtomicBool>) {
    let state = app.state::<Shared>();
    let mut batch = first;
    loop {
        run_job(&app, &batch, &cancel);
        let next = {
            // Trava a mesma ordem que launch(): quem chega agora ou entra na fila ou acha o trabalho livre.
            let mut running = state.cancel.lock().unwrap();
            let next = if cancel.load(Ordering::Relaxed) { None } else { state.queue.lock().unwrap().pop_front() };
            if next.is_none() {
                let left: Vec<Batch> = state.queue.lock().unwrap().drain(..).collect();
                for item in left.iter().flat_map(|b| b.items.iter()) {
                    let _ = app.emit("job", JobEvent::Cancelled { id: item.id });
                }
                *running = None;
                state.downloading.store(false, Ordering::SeqCst);
                state.skip.lock().unwrap().clear();
            }
            next
        };
        match next {
            Some(b) => batch = b,
            None => break,
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some("Frey Project"));
    }
}

fn history_path(app: &AppHandle) -> PathBuf {
    config_dir(app).unwrap_or_default().join("baixados.json")
}

fn with_history<T>(app: &AppHandle, change: impl FnOnce(&mut Vec<history::Entry>) -> T) -> T {
    let state = app.state::<Shared>();
    let mut guard = state.history.lock().unwrap();
    let list = guard.get_or_insert_with(|| history::load(&history_path(app)));
    let result = change(list);
    history::save(&history_path(app), list);
    result
}

fn record(app: &AppHandle, item: &JobItem, folder: &Path, files: &[PathBuf], state: &str, message: String) {
    let bytes = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    let entry = history::Entry {
        id: format!("{}-{}", history::now(), item.id),
        url: item.url.clone(),
        kind: item.kind.clone(),
        // Modo rapido comeca antes da previa: sem titulo, vale o nome do arquivo baixado.
        title: if !item.title.is_empty() {
            item.title.clone()
        } else {
            files.first().filter(|f| f.is_file()).and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| item.url.clone())
        },
        thumb: item.thumb.clone(),
        folder: folder.display().to_string(),
        files: files.iter().map(|f| f.display().to_string()).collect(),
        state: state.into(),
        message,
        quality: item.quality.clone(),
        bytes,
        when: history::now(),
    };
    with_history(app, |list| history::add(list, entry.clone()));
    let _ = app.emit("history", entry);
}

fn run_job(app: &AppHandle, batch: &Batch, cancel: &Arc<AtomicBool>) {
    let state = app.state::<Shared>();
    let client = drive::Client::new();
    let (mut ok, mut failed) = (0, 0);
    let total = batch.items.len();
    let ui_lang = language(app);
    for (position, item) in batch.items.iter().enumerate() {
        let target = if item.folder.is_empty() { batch.base.clone() } else { batch.base.join(models::rel_path(&item.folder)) };
        if cancel.load(Ordering::Relaxed) || state.skip.lock().unwrap().remove(&item.id) {
            let _ = app.emit("job", JobEvent::Cancelled { id: item.id });
            if !cancel.load(Ordering::Relaxed) {
                record(app, item, &target, &[], "cancelled", String::new());
            }
            continue;
        }
        // Parar este link nao para os outros: cada um tem o proprio sinal, e cancelar tudo marca os dois.
        let item_cancel = Arc::new(AtomicBool::new(false));
        *state.item_cancel.lock().unwrap() = Some((item.id, item_cancel.clone()));
        let _ = app.emit("job", JobEvent::Start { id: item.id });
        let link = drive::Link { url: item.url.clone(), kind: if item.kind == "folder" { "folder" } else { "file" } };
        let mut last_emit = Instant::now();
        let mut on_progress = |progress: drive::Progress| {
            if last_emit.elapsed().as_millis() >= 150 {
                last_emit = Instant::now();
                if let Some(tray) = app.tray_by_id(TRAY_ID) {
                    let percent = progress.total.filter(|t| *t > 0).map(|t| progress.done * 100 / t);
                    let text = match percent {
                        Some(p) => format!("Frey Project: {}/{total}, {p}%", position + 1),
                        None => format!("Frey Project: {}/{total}", position + 1),
                    };
                    let _ = tray.set_tooltip(Some(text));
                }
                let _ = app.emit("job", JobEvent::Progress { id: item.id, progress });
            }
        };
        let mut edit_error = None;
        let extra = youtube::Extra { subs: item.subs.then(|| subtitle_langs(&ui_lang)) };
        let section = match (item.start, item.end) {
            (Some(start), Some(end)) if end > start => Some((start, end)),
            _ => None,
        };
        let result = if youtube::handles(&item.kind) && section.is_some() {
            download_section(app, item, &target, section.unwrap_or_default(), &extra, &item_cancel, &mut on_progress)
        } else if youtube::handles(&item.kind) && item.quality == "edit4k" && extra == youtube::Extra::default() {
            download_for_editing(app, item.id, &item.url, &target, &item_cancel, &mut on_progress, &mut edit_error)
        } else if youtube::handles(&item.kind) {
            let logins = logins_for(app, &item.url);
            let quality = if item.quality == "edit4k" { "best" } else { item.quality.as_str() };
            youtube::download_with(&item.url, &target, quality, &extra, &logins, &mut |login| remember_login(app, &item.url, login), &item_cancel, &mut on_progress)
        } else {
            client.download_link(&link, &target, &item_cancel, &mut on_progress)
        };
        *state.item_cancel.lock().unwrap() = None;
        match result {
            Ok(outcome) => {
                ok += 1;
                // "4K para edicao" com trecho ou legenda baixou no melhor ate 4K: a conversao entrega H.264.
                let special = section.is_some() || extra != youtube::Extra::default();
                let convert_to = if item.quality == "edit4k" && special { "mp4".to_string() } else { batch.convert_to.clone() };
                let (converted, convert_error) = if convert_to.is_empty() {
                    (0, None)
                } else {
                    convert_downloaded(app, item.id, &outcome.files, &convert_to, &item_cancel)
                };
                let convert_error = edit_error.or(convert_error);
                let mut files = outcome.files.clone();
                if files.is_empty() {
                    files.push(target.clone());
                }
                let state_name = if convert_error.is_some() { "warning" } else { "done" };
                record(app, item, &target, &files, state_name, convert_error.clone().unwrap_or_default());
                let _ = app.emit("job", JobEvent::Ok { id: item.id, downloaded: outcome.downloaded, skipped: outcome.skipped, converted, convert_error });
            }
            Err(drive::Error::Cancelled) => {
                record(app, item, &target, &[], "cancelled", String::new());
                let _ = app.emit("job", JobEvent::Cancelled { id: item.id });
            }
            Err(drive::Error::Message(message)) => {
                failed += 1;
                record(app, item, &target, &[], "fail", message.clone());
                let _ = app.emit("job", JobEvent::Fail { id: item.id, message });
            }
        }
    }

    let cancelled = cancel.load(Ordering::Relaxed);
    let more = !cancelled && !state.queue.lock().unwrap().is_empty();
    let _ = app.emit("job", JobEvent::Done { failed, ok, cancelled, base: batch.base.display().to_string(), more });
    if !cancelled && failed == 0 && batch.open_at_end {
        open_folder(&batch.base);
    }
    if !cancelled && !more && !panel(app).is_visible().unwrap_or(false) {
        let key = if failed == 0 { "notifyDownloaded" } else { "notifyDownloadFailed" };
        let body = i18n::native(&ui_lang, key).replace("{0}", &ok.to_string()).replace("{1}", &failed.to_string());
        let _ = app.notification().builder().title("Frey Project").body(body).show();
    }
}

/// Nome do trecho: "Titulo (2.50-6.55).mp4". O video inteiro, ou outro trecho, nao e confundido com este.
fn section_name(file: &Path, start: f64, end: f64) -> String {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let label = format!("{}-{}", youtube::clock(start), youtube::clock(end)).replace(':', ".");
    match file.extension() {
        Some(ext) => format!("{stem} ({label}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({label})"),
    }
}

/// So um trecho: baixa o video numa pasta temporaria ao lado do destino e corta no PC, sem recodificar.
/// O --download-sections do yt-dlp fica parado lendo o video direto do YouTube, por isso nao e usado.
/// A pasta temporaria tem nome fixo por link e trecho: cancelar e tentar de novo continua de onde parou.
fn download_section(
    app: &AppHandle,
    item: &JobItem,
    target: &Path,
    (start, end): (f64, f64),
    extra: &youtube::Extra,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(drive::Progress),
) -> Result<drive::Outcome, drive::Error> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(&item.url, &item.quality, start.to_bits(), end.to_bits()), &mut hasher);
    let temp = target.join(format!(".frey-trecho-{:016x}", std::hash::Hasher::finish(&hasher)));
    let logins = logins_for(app, &item.url);
    let quality = if item.quality == "edit4k" { "best" } else { item.quality.as_str() };
    youtube::download_with(&item.url, &temp, quality, extra, &logins, &mut |login| remember_login(app, &item.url, login), cancel, on_progress)?;

    let mut outcome = drive::Outcome { downloaded: 0, skipped: 0, files: Vec::new() };
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&temp)?.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_file()).collect();
    sources.sort();
    for source in sources {
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if convert::is_media(&source) {
            let dst = target.join(section_name(&source, start, end));
            if dst.exists() {
                outcome.skipped += 1;
                continue;
            }
            match convert::cut(&source, &dst, start, end, cancel) {
                Ok(()) => {
                    outcome.downloaded += 1;
                    outcome.files.push(dst);
                }
                Err(convert::Error::Cancelled) => return Err(drive::Error::Cancelled),
                Err(convert::Error::Message(m)) => return Err(drive::Error::Message(i18n::code("err.cutFailed", &[&m]))),
            }
        } else if name.to_lowercase().ends_with(".srt") {
            // A legenda vem inteira, com os tempos do video original.
            let dst = target.join(&name);
            if !dst.exists() {
                let _ = std::fs::rename(&source, &dst);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&temp);
    if outcome.downloaded + outcome.skipped == 0 {
        return Err(drive::Error::Message("err.ytNoFile".into()));
    }
    Ok(outcome)
}

/// Legenda no idioma do painel e em ingles, as mais comuns de existir.
fn subtitle_langs(lang: &str) -> String {
    let code = match lang {
        "zht" => "zh-Hant",
        "zh" => "zh-Hans",
        "" => "pt",
        other => other,
    };
    if code == "en" { "en.*".into() } else { format!("{code}.*,en.*") }
}

// ---------------------------------------------------------------- previa dos links

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeAnswer {
    meta: Option<youtube::Meta>,
    /// Codigo de erro (err.*) quando a previa falhou.
    error: Option<String>,
    /// "youtube" ou "instagram" quando o site aceita o login opcional.
    site: Option<&'static str>,
    /// Onde a conta foi achada: "firefox", "brave"... ou "frey".
    login_via: Option<String>,
    risk_accepted: bool,
    /// Navegadores instalados que o Frey olhou.
    browsers: Vec<&'static str>,
}

#[tauri::command(async)]
fn probe_link(app: AppHandle, url: String) -> ProbeAnswer {
    let logins = logins_for(&app, &url);
    let risk_accepted = app.state::<Shared>().settings.lock().unwrap().youtube_risco_aceito;
    let site = youtube::site(&url);
    let browsers = youtube::installed_browsers();
    match youtube::probe(&url, &logins) {
        Ok((meta, used)) => {
            if let Some(login) = &used {
                remember_login(&app, &url, login);
            }
            ProbeAnswer { meta: Some(meta), error: None, site, login_via: used.map(|l| login_label(&app, &l)), risk_accepted, browsers }
        }
        Err(drive::Error::Message(code)) => ProbeAnswer { meta: None, error: Some(code), site, login_via: None, risk_accepted, browsers },
        Err(drive::Error::Cancelled) => ProbeAnswer { meta: None, error: None, site, login_via: None, risk_accepted, browsers },
    }
}

// ---------------------------------------------------------------- aba Baixados

#[tauri::command(async)]
fn history_list(app: AppHandle) -> Vec<history::Entry> {
    with_history(&app, |list| list.clone())
}

#[tauri::command(async)]
fn history_remove(app: AppHandle, id: String) {
    with_history(&app, |list| history::remove(list, &id));
}

#[tauri::command(async)]
fn history_clear_done(app: AppHandle) -> Vec<history::Entry> {
    with_history(&app, |list| {
        history::clear_done(list);
        list.clone()
    })
}

/// Abre o arquivo no programa padrao do sistema.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err("err.fileGone".into());
    }
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    command.arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Mostra o arquivo selecionado na pasta.
#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return match path.parent().filter(|p| p.is_dir()) {
            Some(parent) => {
                open_folder(parent);
                Ok(())
            }
            None => Err("err.fileGone".into()),
        };
    }
    if path.is_dir() {
        open_folder(&path);
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- capas dos cartoes

/// Base64 sem dependencia extra: as capas chegam ao painel como data:, e a politica de conteudo
/// continua sem liberar imagens de outros sites.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Tipo da imagem pelos primeiros bytes; outra coisa nao vira capa.
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        _ => None,
    }
}

static THUMBS: std::sync::OnceLock<Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();

/// Capa de um video como data: (com cache na memoria). Vazio quando nao da para baixar.
#[tauri::command(async)]
fn thumb_data(url: String) -> String {
    if !url.starts_with("https://") {
        return String::new();
    }
    let cache = THUMBS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(found) = cache.lock().unwrap().get(&url) {
        return found.clone();
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .build()
        .into();
    let bytes = agent
        .get(&url)
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().with_config().limit(4 * 1024 * 1024).read_to_vec().ok())
        .unwrap_or_default();
    let data = match image_mime(&bytes) {
        Some(mime) => format!("data:{mime};base64,{}", base64(&bytes)),
        None => String::new(),
    };
    let mut cache = cache.lock().unwrap();
    if cache.len() > 300 {
        cache.clear();
    }
    cache.insert(url, data.clone());
    data
}

#[cfg(test)]
mod thumb_tests {
    use super::*;

    #[test]
    fn base64_e_tipo_da_imagem() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_mime(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_mime(b"<html>"), None, "pagina de erro nao vira capa");
    }

    #[test]
    fn login_do_frey_reconhece_a_sessao_de_cada_site() {
        let dir = std::env::temp_dir().join(format!("frey-login-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("login.txt");
        std::fs::write(&path, "# Netscape HTTP Cookie File\n.instagram.com\tTRUE\t/\tTRUE\t0\tcsrftoken\tx\n").unwrap();
        assert!(login_state_of(&path).is_empty(), "so o csrftoken nao e login");
        std::fs::write(&path, ".instagram.com\tTRUE\t/\tTRUE\t0\tsessionid\tabc\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\txyz\n").unwrap();
        assert_eq!(login_state_of(&path), vec!["youtube", "instagram"]);
        assert!(login_state_of(&dir.join("nao-existe.txt")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trecho_entra_no_nome_do_arquivo() {
        assert_eq!(section_name(Path::new("C:/x/Big Buck Bunny.mp4"), 170.0, 415.4), "Big Buck Bunny (2.50-6.55).mp4");
        assert_eq!(section_name(Path::new("Reel [a].webm"), 5.0, 3725.0), "Reel [a] (0.05-1.02.05).webm");
    }

    #[test]
    fn legenda_no_idioma_do_painel_e_em_ingles() {
        assert_eq!(subtitle_langs("pt"), "pt.*,en.*");
        assert_eq!(subtitle_langs("en"), "en.*");
        assert_eq!(subtitle_langs("zht"), "zh-Hant.*,en.*");
    }
}

// ---------------------------------------------------------------- "Entrar pelo Frey"

/// Sites com sessao no arquivo de login do Frey, pelos cookies que so existem com a conta logada.
fn login_state_of(path: &Path) -> Vec<&'static str> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let has = |domain: &str, names: &[&str]| {
        text.lines().filter(|l| !l.starts_with('#') || l.starts_with("#HttpOnly_")).any(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            parts.len() >= 7 && parts[0].trim_start_matches("#HttpOnly_").ends_with(domain) && names.contains(&parts[5]) && !parts[6].is_empty()
        })
    };
    let mut sites = Vec::new();
    if has("youtube.com", &["SAPISID", "__Secure-3PSID", "LOGIN_INFO"]) {
        sites.push("youtube");
    }
    if has("instagram.com", &["sessionid"]) {
        sites.push("instagram");
    }
    sites
}

#[tauri::command]
fn login_state(app: AppHandle) -> Vec<&'static str> {
    login_state_of(&frey_login_path(&app))
}

#[tauri::command(async)]
fn forget_frey_login(app: AppHandle) -> Vec<&'static str> {
    let _ = std::fs::remove_file(frey_login_path(&app));
    if let Some(window) = app.get_webview_window("login") {
        let _ = window.destroy();
    }
    // Apaga tambem a sessao guardada pelo WebView, para a proxima entrada comecar do zero.
    if let Ok(cookies) = panel(&app).cookies() {
        for cookie in cookies {
            let domain = cookie.domain().unwrap_or("").to_string();
            if ["instagram.com", "youtube.com", "google.com"].iter().any(|d| domain.ends_with(d)) {
                let _ = panel(&app).delete_cookie(cookie);
            }
        }
    }
    login_state_of(&frey_login_path(&app))
}

const LOGIN_PAGES: [&str; 4] = ["https://www.instagram.com/", "https://www.youtube.com/", "https://accounts.google.com/", "https://www.google.com/"];

/// Escreve os cookies da janela de login no formato que o yt-dlp le (Netscape).
fn save_frey_login(app: &AppHandle, window: &WebviewWindow) -> Vec<&'static str> {
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string(), "# Login feito pela janela do Frey Project. Nao compartilhe este arquivo.".to_string()];
    let mut seen = std::collections::HashSet::new();
    for page in LOGIN_PAGES {
        let Ok(url) = page.parse() else { continue };
        let Ok(cookies) = window.cookies_for_url(url) else { continue };
        for cookie in cookies {
            let host = page.trim_start_matches("https://").trim_end_matches('/');
            let domain = cookie.domain().map(|d| if d.starts_with('.') { d.to_string() } else { format!(".{d}") }).unwrap_or_else(|| host.to_string());
            let path = cookie.path().unwrap_or("/").to_string();
            if !seen.insert((domain.clone(), path.clone(), cookie.name().to_string())) {
                continue;
            }
            let expires = cookie.expires_datetime().map(|d| d.unix_timestamp().max(0)).unwrap_or(0);
            let secure = if cookie.secure().unwrap_or(false) { "TRUE" } else { "FALSE" };
            let sub = if domain.starts_with('.') { "TRUE" } else { "FALSE" };
            lines.push(format!("{domain}\t{sub}\t{path}\t{secure}\t{expires}\t{}\t{}", cookie.name(), cookie.value()));
        }
    }
    let path = frey_login_path(app);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
    login_state_of(&path)
}

/// Abre a pagina de login do site numa janela do proprio Frey. Ao fechar, guarda o login para o yt-dlp.
/// Assincrono: criar janela num comando sincrono trava a thread principal no Windows.
#[tauri::command(async)]
fn open_login(app: AppHandle, site: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("login") {
        let _ = window.set_focus();
        return Ok(());
    }
    let (url, title) = match site.as_str() {
        "instagram" => ("https://www.instagram.com/accounts/login/", "Instagram"),
        _ => ("https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F", "YouTube"),
    };
    let lang = language(&app);
    let window = WebviewWindowBuilder::new(&app, "login", tauri::WebviewUrl::External(url.parse().map_err(|_| "url")?))
        .title(format!("{} · {title}", i18n::native(&lang, "loginWindow")))
        .inner_size(480.0, 760.0)
        .center()
        .always_on_top(true)
        .build()
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    let closing = window.clone();
    let busy = Arc::new(AtomicBool::new(false));
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if busy.swap(true, Ordering::SeqCst) {
                return;
            }
            // Ler os cookies na thread da janela trava o WebView2: fica numa thread propria.
            let (app, window) = (handle.clone(), closing.clone());
            std::thread::spawn(move || {
                let sites = save_frey_login(&app, &window);
                let _ = window.destroy();
                let _ = app.emit("login-saved", sites);
            });
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_job(state: State<Shared>) {
    if let Some(cancel) = state.cancel.lock().unwrap().as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
    if let Some((_, item)) = state.item_cancel.lock().unwrap().as_ref() {
        item.store(true, Ordering::Relaxed);
    }
}

/// Para so um link: o que esta baixando agora, ou um que ainda esta na fila.
#[tauri::command]
fn cancel_item(state: State<Shared>, id: u32) {
    if let Some((current, item)) = state.item_cancel.lock().unwrap().as_ref() {
        if *current == id {
            item.store(true, Ordering::Relaxed);
            return;
        }
    }
    state.skip.lock().unwrap().insert(id);
}

#[tauri::command]
fn open_project(state: State<Shared>) {
    if let Some((_, base)) = state.last_project.lock().unwrap().as_ref() {
        open_folder(base);
    }
}

// ---------------------------------------------------------------- conversor

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConvertTools {
    ffmpeg: bool,
    gpu: Option<String>,
}

#[tauri::command(async)]
fn convert_tools(app: AppHandle) -> ConvertTools {
    ConvertTools { ffmpeg: convert::find_tool("ffmpeg").is_some(), gpu: cached_gpu(&app) }
}

/// Fica so com arquivos: pastas soltas na lista nao sao convertidas.
#[tauri::command]
fn only_files(paths: Vec<String>) -> Vec<String> {
    paths.into_iter().filter(|p| Path::new(p).is_file()).collect()
}

#[tauri::command]
fn is_folder(path: String) -> bool {
    Path::new(&path).is_dir()
}

#[tauri::command(async)]
fn pick_files(app: AppHandle, state: State<'_, Shared>) -> Vec<String> {
    state.dialog_open.store(true, Ordering::SeqCst);
    let picked = app
        .dialog()
        .file()
        .set_title(i18n::native(&language(&app), "dialogFiles"))
        .set_parent(&panel(&app))
        .add_filter(i18n::native(&language(&app), "filterMedia"), &["mkv", "mp4", "mov", "avi", "webm", "flv", "wmv", "m4v", "ts", "mts", "m2ts", "3gp", "mpg", "mp3", "wav", "flac", "m4a", "ogg", "aac"])
        .add_filter(i18n::native(&language(&app), "filterAll"), &["*"])
        .blocking_pick_files();
    state.dialog_open.store(false, Ordering::SeqCst);
    let _ = panel(&app).set_focus();
    picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
        .collect()
}

#[derive(Deserialize)]
struct ConvertFile {
    id: u32,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertRequest {
    files: Vec<ConvertFile>,
    ext: String,
    mode: String,
    crf: u32,
    use_gpu: bool,
    out_dir: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum ConvertEvent {
    Start { id: u32 },
    Progress { id: u32, percent: f64, how: String },
    Ok { id: u32, output: String },
    Fail { id: u32, message: String },
    Cancelled { id: u32 },
    Done { ok: usize, failed: usize, cancelled: bool },
}

#[tauri::command]
fn start_convert(app: AppHandle, state: State<Shared>, request: ConvertRequest) -> Result<(), String> {
    if state.cancel.lock().unwrap().is_some() {
        return Err("err.busy".into());
    }
    if !convert::is_output_format(&request.ext) {
        return Err("err.formatInvalid".into());
    }
    if convert::find_tool("ffmpeg").is_none() {
        return Err(convert::FFMPEG_MISSING.into());
    }
    if request.files.is_empty() {
        return Err("err.noFiles".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let worker_app = app.clone();
    std::thread::spawn(move || run_convert(worker_app, request, cancel));
    Ok(())
}

fn run_convert(app: AppHandle, request: ConvertRequest, cancel: Arc<AtomicBool>) {
    let gpu = if request.use_gpu { cached_gpu(&app) } else { None };
    let out_dir = Some(PathBuf::from(request.out_dir.trim())).filter(|p| !p.as_os_str().is_empty());
    let options = convert::Options { ext: request.ext, mode: request.mode, crf: request.crf, gpu, out_dir };
    let (mut ok, mut failed) = (0, 0);
    for file in &request.files {
        if cancel.load(Ordering::Relaxed) {
            let _ = app.emit("convert", ConvertEvent::Cancelled { id: file.id });
            continue;
        }
        let _ = app.emit("convert", ConvertEvent::Start { id: file.id });
        let mut last_emit = Instant::now();
        let mut on_progress = |percent: f64, how: &str| {
            if percent == 0.0 || last_emit.elapsed().as_millis() >= 150 {
                last_emit = Instant::now();
                let _ = app.emit("convert", ConvertEvent::Progress { id: file.id, percent, how: how.to_string() });
            }
        };
        match convert::convert_file(Path::new(&file.path), &options, &cancel, &mut on_progress) {
            Ok(output) => {
                ok += 1;
                *app.state::<Shared>().last_converted.lock().unwrap() = output.parent().map(Path::to_path_buf);
                let _ = app.emit("convert", ConvertEvent::Ok { id: file.id, output: output.display().to_string() });
            }
            Err(convert::Error::Cancelled) => {
                let _ = app.emit("convert", ConvertEvent::Cancelled { id: file.id });
            }
            Err(convert::Error::Message(message)) => {
                failed += 1;
                let _ = app.emit("convert", ConvertEvent::Fail { id: file.id, message });
            }
        }
    }
    let cancelled = cancel.load(Ordering::Relaxed);
    *app.state::<Shared>().cancel.lock().unwrap() = None;
    let _ = app.emit("convert", ConvertEvent::Done { ok, failed, cancelled });
    if !cancelled && !panel(&app).is_visible().unwrap_or(false) {
        let body = i18n::native(&language(&app), "notifyConverted").replace("{0}", &ok.to_string()).replace("{1}", &failed.to_string());
        let _ = app.notification().builder().title("Frey Project").body(body).show();
    }
}

#[tauri::command]
fn open_converted(state: State<Shared>) {
    if let Some(folder) = state.last_converted.lock().unwrap().as_ref() {
        open_folder(folder);
    }
}

#[tauri::command(async)]
fn pick_out_dir(app: AppHandle, state: State<'_, Shared>) -> Option<String> {
    state.dialog_open.store(true, Ordering::SeqCst);
    let picked = app.dialog().file().set_title(i18n::native(&language(&app), "dialogOut")).set_parent(&panel(&app)).blocking_pick_folder();
    state.dialog_open.store(false, Ordering::SeqCst);
    let _ = panel(&app).set_focus();
    picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

// ---------------------------------------------------------------- configuracoes

#[derive(Serialize, Deserialize)]
struct ModelData {
    name: String,
    root: String,
    folders: Vec<String>,
    #[serde(default)]
    destination: String,
    /// Arquivos de texto do modelo: o editor nao mexe, mas nao podem se perder ao salvar.
    #[serde(default)]
    files: Vec<(String, String)>,
}

impl From<&models::Template> for ModelData {
    fn from(t: &models::Template) -> Self {
        ModelData { name: t.name.clone(), root: t.root.clone(), folders: t.folders.clone(), destination: t.destination.clone(), files: t.files.clone() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelsAnswer {
    models: Vec<ModelData>,
    open_at_end: bool,
}

fn models_answer(config: &models::Config) -> ModelsAnswer {
    ModelsAnswer { models: config.templates.iter().map(ModelData::from).collect(), open_at_end: config.open_at_end }
}

#[tauri::command]
fn get_models(app: AppHandle) -> Result<ModelsAnswer, String> {
    models::load_config(&config_path(&app)).map(|c| models_answer(&c))
}

#[tauri::command]
fn save_models(app: AppHandle, models: Vec<ModelData>, open_at_end: bool) -> Result<ModelsAnswer, String> {
    let templates = models
        .into_iter()
        .map(|m| models::Template { name: m.name, root: m.root, folders: m.folders, destination: m.destination, files: m.files })
        .collect();
    let path = config_path(&app);
    let default_name = models::load_config(&path).map(|c| c.default_name).unwrap_or_default();
    let config = models::Config { default_name, open_at_end, templates: models::normalize_templates(templates)? };
    models::save_config(&path, &config)?;
    Ok(models_answer(&config))
}

/// Traz o modelo padrao de volta sem apagar os outros. Arquivo com erro e guardado como .bak antes.
#[tauri::command]
fn restore_default_model(app: AppHandle) -> Result<ModelsAnswer, String> {
    let path = config_path(&app);
    let mut config = match models::load_config(&path) {
        Ok(config) => config,
        Err(_) => {
            let _ = std::fs::rename(&path, path.with_extension("json.bak"));
            models::default_config()
        }
    };
    let default = models::default_template();
    match config.templates.iter_mut().find(|t| t.name == default.name) {
        Some(existing) => *existing = default,
        None => config.templates.insert(0, default),
    }
    models::save_config(&path, &config)?;
    Ok(models_answer(&config))
}

#[tauri::command]
fn set_language(app: AppHandle, state: State<Shared>, language: String) -> Result<(), String> {
    if !i18n::LANGUAGES.contains(&language.as_str()) {
        return Err("err.formatInvalid".into());
    }
    {
        let mut settings = state.settings.lock().unwrap();
        settings.idioma = language.clone();
        save_settings(&app, &settings);
    }
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), tray_menu(&app, &language)) {
        let _ = tray.set_menu(Some(menu));
    }
    Ok(())
}

fn tray_menu(app: &AppHandle, language: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", i18n::native(language, "trayOpen"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", i18n::native(language, "trayQuit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
}

/// Links da copia mais recente, uma unica vez por copia. Chamado ao abrir o painel.
#[tauri::command]
fn take_copied_links(app: AppHandle, state: State<Shared>) -> Vec<drive::Link> {
    let is_new = state.clipboard.take_new();
    if !is_new || state.settings.lock().unwrap().ignorar_link_copiado {
        return Vec::new();
    }
    app.clipboard().read_text().map(|text| clipboard::links_in(&text)).unwrap_or_default()
}

/// Liga ou desliga o login do YouTube. Sem a confirmacao do risco, fica sempre desligado.
#[tauri::command]
fn set_youtube_login(app: AppHandle, state: State<Shared>, source: String, file: String, accepted: bool) {
    let mut settings = state.settings.lock().unwrap();
    settings.youtube_risco_aceito = accepted;
    settings.youtube_login = if accepted { source } else { String::new() };
    settings.youtube_cookies = file;
    save_settings(&app, &settings);
}

#[tauri::command]
fn pick_cookies_file(app: AppHandle, state: State<Shared>) -> Option<String> {
    state.dialog_open.store(true, Ordering::SeqCst);
    let picked = app
        .dialog()
        .file()
        .set_title("cookies.txt")
        .set_parent(&panel(&app))
        .add_filter("cookies.txt", &["txt"])
        .blocking_pick_file();
    state.dialog_open.store(false, Ordering::SeqCst);
    let _ = panel(&app).set_focus();
    picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

#[tauri::command]
fn set_detect_link(app: AppHandle, state: State<Shared>, detect: bool) {
    let mut settings = state.settings.lock().unwrap();
    settings.ignorar_link_copiado = !detect;
    save_settings(&app, &settings);
}

#[tauri::command]
fn set_download_only(app: AppHandle, state: State<Shared>, download_only: bool) {
    let mut settings = state.settings.lock().unwrap();
    settings.so_baixar = download_only;
    save_settings(&app, &settings);
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    let _ = panel(&app).hide();
}

#[tauri::command]
fn resize_panel(app: AppHandle, height: f64) {
    let _ = panel(&app).set_size(LogicalSize::new(PANEL_WIDTH, height.clamp(120.0, 720.0)));
}

// ---------------------------------------------------------------- inicio

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut std::ffi::c_void;
    fn GetWindowThreadProcessId(window: *mut std::ffi::c_void, process: *mut u32) -> u32;
    fn AttachThreadInput(attach: u32, attach_to: u32, enable: i32) -> i32;
}

#[cfg(windows)]
/// O Windows nega o foco a um app em segundo plano, como quando o atalho da Area de Trabalho
/// chama a copia que ja esta na bandeja. Quem pediu o painel foi o usuario: une-se a fila de
/// entrada da janela em primeiro plano so durante o pedido.
fn take_foreground(own: Option<*mut std::ffi::c_void>) {
    let Some(own) = own else { return };
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == own || foreground.is_null() {
            return;
        }
        let own_thread = GetWindowThreadProcessId(own, std::ptr::null_mut());
        let foreground_thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
        let attached = own_thread != foreground_thread && AttachThreadInput(own_thread, foreground_thread, 1) != 0;
        SetForegroundWindow(own);
        if attached {
            AttachThreadInput(own_thread, foreground_thread, 0);
        }
    }
}

/// Durante uma troca o Windows pode nao ter janela em primeiro plano: aguarda definir antes de decidir.
#[cfg(windows)]
fn focus_really_left(window: &tauri::Window) -> bool {
    let Ok(own) = window.hwnd() else { return false };
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let foreground = unsafe { GetForegroundWindow() };
        if foreground == own.0 as *mut _ {
            return false;
        }
        if !foreground.is_null() {
            return true;
        }
    }
    false
}

#[cfg(windows)]
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: *mut std::ffi::c_void, attribute: u32, value: *const std::ffi::c_void, size: u32) -> i32;
}

/// Tira a borda na cor de destaque que o Windows 11 desenha em volta da janela: destoa do vidro.
#[cfg(windows)]
fn remove_accent_border(window: &WebviewWindow) {
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    if let Ok(hwnd) = window.hwnd() {
        let color = DWMWA_COLOR_NONE;
        unsafe {
            DwmSetWindowAttribute(hwnd.0 as *mut _, DWMWA_BORDER_COLOR, &color as *const u32 as *const _, 4);
        }
    }
}

/// So guarda o numero da copia atual, uma vez por segundo. O texto nao e lido aqui.
fn watch_clipboard(app: &AppHandle) {
    app.state::<Shared>().clipboard.start();
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        app.state::<Shared>().clipboard.tick();
    });
}

fn register_hotkey(app: &AppHandle, text: &str) -> Result<(), String> {
    let shortcut: Shortcut = text.parse().map_err(|_| i18n::code("err.hotkeyInvalid", &[text]))?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|_| i18n::code("err.hotkeyBusy", &[text]))
}

/// Coloca as ferramentas instaladas junto com o app na frente do PATH: a pasta bin no Windows,
/// e no Mac a pasta do proprio executavel, onde o pacote .app guarda ffmpeg, yt-dlp e Deno.
/// O yt-dlp tambem acha assim o ffmpeg e o Deno, e a copia embutida vence a do sistema.
fn use_bundled_tools() {
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else { return };
    let bin = if cfg!(target_os = "macos") { dir } else { dir.join("bin") };
    if !bin.is_dir() {
        return;
    }
    let mut paths = vec![bin];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

fn main() {
    use_bundled_tools();
    let mut builder = tauri::Builder::default();
    // Instancia de teste convive com o app instalado, com configuracao propria.
    if std::env::var_os("FREY_PROJECT_TEST_INSTANCE").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|a| a == HIDDEN_ARG) {
                show_panel(app);
            }
        }));
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![HIDDEN_ARG])));
    }
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_panel(app);
                    }
                })
                .build(),
        )
        .manage(Shared::default())
        .invoke_handler(tauri::generate_handler![
            boot, preview, parse_links, pick_folder, start_job, cancel_job, open_project, hide_panel, resize_panel,
            convert_tools, only_files, is_folder, pick_files, pick_out_dir, start_convert, open_converted,
            get_models, save_models, restore_default_model, set_language, set_download_only, take_copied_links, set_detect_link, set_youtube_login, pick_cookies_file,
            recent_paths, forget_path, list_subfolders, cancel_item, probe_link, history_list, history_remove, history_clear_done,
            open_path, reveal_path, open_login, login_state, forget_frey_login, thumb_data
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Perder o foco nao esconde: o painel fica aberto para receber arquivos do Explorer.
            // Logo depois de abrir, quem iniciou o programa pode recuperar o foco por um instante.
            #[cfg(windows)]
            WindowEvent::Focused(false) => {
                let window = window.clone();
                std::thread::spawn(move || {
                    let just_opened = window.state::<Shared>().panel_shown_at.lock().unwrap().is_some_and(|at| at.elapsed() < JUST_OPENED);
                    if just_opened && focus_really_left(&window) {
                        take_foreground(window.hwnd().ok().map(|h| h.0 as *mut std::ffi::c_void));
                    }
                });
            }
            _ => {}
        })
        .setup(|app| {
            let handle = app.handle().clone();
            // Nascer escondida faz o painel recusar arquivos arrastados do Explorer.
            // Por isso a janela nasce visivel, sem foco e alem da borda da tela, e so entao se esconde.
            let config = handle.config().app.windows.iter().find(|w| w.label == "panel").cloned().expect("janela do painel na configuracao");
            #[allow(unused_mut)]
            let mut builder = WebviewWindowBuilder::from_config(&handle, &config)?;
            #[cfg(windows)]
            if let Some((x, y)) = beyond_the_screens(&handle) {
                builder = builder.position(x, y);
            }
            let window = builder.build()?;
            let _ = window.hide();
            #[cfg(windows)]
            remove_accent_border(&window);
            let (busy, closing) = (handle.clone(), window.clone());
            let busy = move || busy.state::<Shared>().dialog_open.load(Ordering::SeqCst);
            let close = move || {
                let _ = closing.hide();
            };
            #[cfg(windows)]
            if let Ok(hwnd) = window.hwnd() {
                outside_click::watch(hwnd.0 as isize, busy, close);
            }
            #[cfg(target_os = "macos")]
            {
                outside_click::watch(window.clone(), busy, close);
                // Como na bandeja do Windows: so o icone na barra de menus, sem icone no Dock.
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                // Iniciar com o Mac. O instalador do Windows ja grava isso no registro.
                use tauri_plugin_autostart::ManagerExt;
                let launcher = handle.autolaunch();
                if !launcher.is_enabled().unwrap_or(true) {
                    let _ = launcher.enable();
                }
            }
            let settings: Settings = settings_path(&handle)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            let hotkey = if settings.atalho.is_empty() { DEFAULT_HOTKEY.to_string() } else { settings.atalho.clone() };
            watch_clipboard(&handle);
            let tray_language = settings.idioma.clone();
            let state = app.state::<Shared>();
            *state.settings.lock().unwrap() = settings;
            if let Err(error) = register_hotkey(&handle, &hotkey) {
                *state.hotkey_error.lock().unwrap() = Some(error);
            }

            let menu = tray_menu(&handle, &tray_language)?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().expect("icone"))
                .tooltip("Frey Project")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_panel(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_panel(tray.app_handle());
                    }
                })
                .build(app)?;
            // Ao iniciar com o Windows, fica so na bandeja: abrir aqui roubaria o foco de quem esta digitando.
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show_panel(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o Frey Project")
        .run(|_app, _event| {
            // No Mac, abrir o app de novo pelo Finder ou pelo Launchpad nao inicia outra copia:
            // o sistema avisa a que ja esta rodando, e ela mostra o painel.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_panel(_app);
            }
        });
}
