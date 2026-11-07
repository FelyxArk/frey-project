// Frey Project: painel residente chamado por atalho global (como o Listary ou a barra do Finder).
// Fica na bandeja, cria a estrutura do projeto e baixa os links do Drive. Nao fala com o Premiere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod convert;
mod drive;
mod i18n;
mod models;
mod outside_click;
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

const DEFAULT_HOTKEY: &str = "ctrl+alt+p";
const PANEL_WIDTH: f64 = 640.0;
const TRAY_ID: &str = "main";
const HIDDEN_ARG: &str = "--hidden";
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
}

#[derive(Default)]
struct Shared {
    settings: Mutex<Settings>,
    hotkey_error: Mutex<Option<String>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// (modelo|nome|destino, pasta do projeto): tentar de novo nao pergunta outra vez.
    last_project: Mutex<Option<(String, PathBuf)>>,
    last_converted: Mutex<Option<PathBuf>>,
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
fn open_folder(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || match find_open_folder(&path) {
        Some(window) => bring_to_front(window),
        None => {
            let _ = std::process::Command::new("explorer.exe").arg(&path).spawn();
        }
    });
}

const FIND_FOLDER_SCRIPT: &str = "$want=[IO.Path]::GetFullPath($env:FREY_OPEN_PATH).TrimEnd('\\');foreach($w in (New-Object -ComObject Shell.Application).Windows()){try{$p=$w.Document.Folder.Self.Path;if($p -and $p.TrimEnd('\\') -ieq $want){[int64]$w.HWND;break}}catch{}}";

/// Janela do Explorer ja aberta nesta pasta. Na duvida devolve None, e abre uma janela nova como antes.
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

#[link(name = "user32")]
extern "system" {
    fn IsIconic(window: *mut std::ffi::c_void) -> i32;
    fn ShowWindow(window: *mut std::ffi::c_void, command: i32) -> i32;
    fn SetForegroundWindow(window: *mut std::ffi::c_void) -> i32;
}

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
}

#[tauri::command]
fn boot(app: AppHandle, state: State<Shared>) -> Boot {
    let settings = state.settings.lock().unwrap().clone();
    let config = models::load_config(&config_path(&app));
    let language = settings.idioma.clone();
    let visible = panel(&app).is_visible().unwrap_or(false);
    let download_only = settings.so_baixar;
    let detect_link = !settings.ignorar_link_copiado;
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
            Boot { models: names, default_name: cfg.default_name, model, dest, hotkey, hotkey_error, config_error: None, running, language, visible, download_only, detect_link }
        }
        Err(e) => Boot {
            models: Vec::new(),
            default_name: String::new(),
            model: String::new(),
            dest: String::new(),
            hotkey,
            hotkey_error,
            config_error: Some(e),
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

#[derive(Deserialize)]
struct JobItem {
    id: u32,
    url: String,
    kind: String,
    folder: String,
    /// So para links do YouTube.
    #[serde(default)]
    quality: String,
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
    status: &'static str, // needs_confirm | created | started
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
    Done { failed: usize, ok: usize, cancelled: bool, base: String },
}

#[tauri::command]
fn start_job(app: AppHandle, state: State<Shared>, request: StartRequest) -> Result<StartAnswer, String> {
    if state.cancel.lock().unwrap().is_some() {
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

    if request.items.iter().any(|i| i.kind == "youtube") && convert::find_tool("yt-dlp").is_none() {
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
        save_settings(&app, &settings);
    }

    let answer_base = created.display().to_string();
    if request.items.is_empty() {
        if config.open_at_end {
            open_folder(&created);
        }
        return Ok(StartAnswer { status: "created", root: plan.root, base: answer_base });
    }

    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let open_at_end = config.open_at_end;
    let worker_app = app.clone();
    let convert_to = request.convert_to;
    std::thread::spawn(move || run_job(worker_app, request.items, created, cancel, open_at_end, convert_to));
    Ok(StartAnswer { status: "started", root: plan.root, base: answer_base })
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
    if request.items.iter().any(|i| i.kind == "youtube") && convert::find_tool("yt-dlp").is_none() {
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
        save_settings(&app, &settings);
    }
    // Modelos com erro nao impedem baixar: nesse caso vale abrir a pasta ao terminar.
    let open_at_end = models::load_config(&config_path(&app)).map(|c| c.open_at_end).unwrap_or(true);
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let items: Vec<JobItem> = request.items.into_iter().map(|item| JobItem { folder: String::new(), ..item }).collect();
    let base = dest.clone();
    let convert_to = request.convert_to;
    let worker_app = app.clone();
    std::thread::spawn(move || run_job(worker_app, items, base, cancel, open_at_end, convert_to));
    Ok(StartAnswer { status: "started", root: String::new(), base: dest.display().to_string() })
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
    youtube::download(url, &temp, "edit4k", cancel, on_progress)?;

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

fn run_job(app: AppHandle, items: Vec<JobItem>, base: PathBuf, cancel: Arc<AtomicBool>, open_at_end: bool, convert_to: String) {
    let client = drive::Client::new();
    let (mut ok, mut failed) = (0, 0);
    let total = items.len();
    for (position, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            let _ = app.emit("job", JobEvent::Cancelled { id: item.id });
            continue;
        }
        let _ = app.emit("job", JobEvent::Start { id: item.id });
        let target = if item.folder.is_empty() { base.clone() } else { base.join(&item.folder) };
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
        let result = if item.kind == "youtube" && item.quality == "edit4k" {
            download_for_editing(&app, item.id, &item.url, &target, &cancel, &mut on_progress, &mut edit_error)
        } else if item.kind == "youtube" {
            youtube::download(&item.url, &target, &item.quality, &cancel, &mut on_progress)
        } else {
            client.download_link(&link, &target, &cancel, &mut on_progress)
        };
        match result {
            Ok(outcome) => {
                ok += 1;
                let (converted, convert_error) = if convert_to.is_empty() {
                    (0, None)
                } else {
                    convert_downloaded(&app, item.id, &outcome.files, &convert_to, &cancel)
                };
                let convert_error = edit_error.or(convert_error);
                let _ = app.emit("job", JobEvent::Ok { id: item.id, downloaded: outcome.downloaded, skipped: outcome.skipped, converted, convert_error });
            }
            Err(drive::Error::Cancelled) => {
                let _ = app.emit("job", JobEvent::Cancelled { id: item.id });
            }
            Err(drive::Error::Message(message)) => {
                failed += 1;
                let _ = app.emit("job", JobEvent::Fail { id: item.id, message });
            }
        }
    }

    let cancelled = cancel.load(Ordering::Relaxed);
    *app.state::<Shared>().cancel.lock().unwrap() = None;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some("Frey Project"));
    }
    let _ = app.emit("job", JobEvent::Done { failed, ok, cancelled, base: base.display().to_string() });
    if !cancelled && failed == 0 && open_at_end {
        open_folder(&base);
    }
    if !cancelled && !panel(&app).is_visible().unwrap_or(false) {
        let lang = language(&app);
        let key = if failed == 0 { "notifyDownloaded" } else { "notifyDownloadFailed" };
        let body = i18n::native(&lang, key).replace("{0}", &ok.to_string()).replace("{1}", &failed.to_string());
        let _ = app.notification().builder().title("Frey Project").body(body).show();
    }
}

#[tauri::command]
fn cancel_job(state: State<Shared>) {
    if let Some(cancel) = state.cancel.lock().unwrap().as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
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

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut std::ffi::c_void;
    fn GetWindowThreadProcessId(window: *mut std::ffi::c_void, process: *mut u32) -> u32;
    fn AttachThreadInput(attach: u32, attach_to: u32, enable: i32) -> i32;
}

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

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: *mut std::ffi::c_void, attribute: u32, value: *const std::ffi::c_void, size: u32) -> i32;
}

/// Tira a borda na cor de destaque que o Windows 11 desenha em volta da janela: destoa do vidro.
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

/// Coloca as ferramentas instaladas junto com o app (pasta bin) na frente do PATH.
/// O yt-dlp tambem acha assim o ffmpeg e o Deno, e a copia embutida vence a do sistema.
fn use_bundled_tools() {
    let Some(bin) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("bin"))) else { return };
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
            get_models, save_models, restore_default_model, set_language, set_download_only, take_copied_links, set_detect_link
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Perder o foco nao esconde: o painel fica aberto para receber arquivos do Explorer.
            // Logo depois de abrir, quem iniciou o programa pode recuperar o foco por um instante.
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
            let mut builder = WebviewWindowBuilder::from_config(&handle, &config)?;
            if let Some((x, y)) = beyond_the_screens(&handle) {
                builder = builder.position(x, y);
            }
            let window = builder.build()?;
            let _ = window.hide();
            remove_accent_border(&window);
            if let Ok(hwnd) = window.hwnd() {
                let (busy, closing) = (handle.clone(), window.clone());
                outside_click::watch(
                    hwnd.0 as isize,
                    move || busy.state::<Shared>().dialog_open.load(Ordering::SeqCst),
                    move || {
                        let _ = closing.hide();
                    },
                );
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
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Frey Project");
}
