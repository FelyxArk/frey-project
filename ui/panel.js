"use strict";
// Painel do Frey Project. Toda a logica de disco, download e conversao fica no Rust; aqui so ha exibicao.
(function () {
  var invoke = window.__TAURI__.core.invoke;
  var listen = window.__TAURI__.event.listen;
  var STRINGS = window.FreyStrings;
  var $ = function (id) { return document.getElementById(id); };
  var LANGUAGES = [["pt", "Português", "pt-BR"], ["en", "English", "en"], ["es", "Español", "es"], ["fr", "Français", "fr"], ["ja", "日本語", "ja"], ["zh", "简体中文", "zh-Hans"], ["zht", "繁體中文", "zh-Hant"], ["ru", "Русский", "ru"], ["ar", "العربية", "ar"]];
  var FORMATS = [["mp4", "MP4", "fmtVideo"], ["mov", "MOV", "fmtVideo"], ["mkv", "MKV", "fmtVideo"], ["webm", "WEBM", "fmtVideo"], ["gif", "GIF", "fmtAnimated"], ["mp3", "MP3", "fmtAudio"], ["wav", "WAV", "fmtAudio"], ["m4a", "M4A", "fmtAudio"], ["flac", "FLAC", "fmtAudio"], ["opus", "Opus", "fmtAudio"]];
  var QUALITIES = [["1080", "q1080"], ["720", "q720"], ["2160", "q2160"], ["1440", "q1440"], ["edit4k", "qEdit4k"], ["best", "qBest"], ["audio", "qAudioFmt", "M4A"], ["audio-mp3", "qAudioFmt", "MP3"], ["audio-wav", "qAudioFmt", "WAV"], ["audio-flac", "qAudioFmt", "FLAC"], ["audio-opus", "qAudioFmt", "Opus"]];
  var VIDEO_KINDS = { youtube: "kindYoutube", x: "kindX", instagram: "kindInstagram", tiktok: "kindTiktok" };
  var SEP = "\u001f";

  var ui = {
    shell: $("shell"), name: $("name"), model: $("model"), dest: $("dest"), tree: $("tree"), links: $("links"),
    rows: $("rows"), status: $("status"), go: $("go"), cancel: $("cancel"), open: $("open"), hide: $("hide"), actions: $("actions"),
    track: $("progressTrack"), bar: $("progressBar"), hotkey: $("hotkey"), convertTo: $("convertTo"),
    createFolders: $("createFolders"), createFoldersHint: $("createFoldersHint"), quick: $("quick"), queryRow: $("queryRow"), fieldModel: $("fieldModel"), destLabel: $("destLabel"),
    tabProject: $("tabProject"), tabDownloads: $("tabDownloads"), tabConvert: $("tabConvert"), tabSettings: $("tabSettings"),
    viewProject: $("viewProject"), viewDownloads: $("viewDownloads"), viewConvert: $("viewConvert"), viewSettings: $("viewSettings"),
    dlBadge: $("dlBadge"), dlSearch: $("dlSearch"), dlFilters: $("dlFilters"), dlList: $("dlList"), dlEmpty: $("dlEmpty"), clearDone: $("clearDone"),
    sFreyState: $("sFreyState"), sFreyYoutube: $("sFreyYoutube"), sFreyInstagram: $("sFreyInstagram"), sFreyForget: $("sFreyForget"),
    cFormat: $("cFormat"), cMode: $("cMode"), cQuality: $("cQuality"), cOut: $("cOut"), cGpu: $("cGpu"), cGpuLabel: $("cGpuLabel"), cAdd: $("cAdd"), cRows: $("cRows"),
    sLanguage: $("sLanguage"), sOpenAtEnd: $("sOpenAtEnd"), sDetectLink: $("sDetectLink"), sYtRisk: $("sYtRisk"), sYtLogin: $("sYtLogin"), sYtCookies: $("sYtCookies"), sYtCookiesPath: $("sYtCookiesPath"), sModel: $("sModel"), sNew: $("sNew"), sDuplicate: $("sDuplicate"), sDelete: $("sDelete"),
    destMore: $("destMore"), recent: $("recent"), recentList: $("recentList"), recentEmpty: $("recentEmpty"),
    sName: $("sName"), sRoot: $("sRoot"), sFolderList: $("sFolderList"), sAddFolder: $("sAddFolder"), sTree: $("sTree"), sRestore: $("sRestore")
  };
  var s = {
    live: new Map(), partOf: {}, history: [], dlFilter: "all", quick: false, runningKind: "", freySites: [],
    lang: "pt", tab: "project", running: false, projectOpen: false, convertOpen: false, confirmNext: false, broken: false,
    rows: [], files: [], nextId: 1, folders: [], defaultFolder: "", dest: "", outDir: "", requestSeq: 0, settingsSeq: 0, toolsChecked: false, gpu: "detectingGpu",
    status: "", statusKind: "", subfolders: [], subSeq: 0, recent: [], recentOpen: false, root: "", downloadOnly: false, needsName: true, models: [], savedNames: [], modelIndex: 0, saveTimer: null, armed: null
  };

  // ---- traducao

  function t(key) {
    var text = STRINGS[s.lang][key];
    if (text == null) return "[[" + key + "]]";
    if (s.mac) text = text.replace(/Ctrl\+/g, "⌘");
    var args = arguments;
    return text.replace(/\{(\d)\}/g, function (all, n) { return args[Number(n) + 1] != null ? tr(String(args[Number(n) + 1])) : all; });
  }

  // Mensagem vinda do Rust ou guardada aqui: "chave", "chave<SEP>arg", ou texto cru (ffmpeg, caminhos).
  function tr(message) {
    if (!message) return "";
    var parts = String(message).split(SEP);
    if (!Object.prototype.hasOwnProperty.call(STRINGS[s.lang], parts[0])) return String(message);
    return t.apply(null, parts);
  }

  function c() { return Array.prototype.join.call(arguments, SEP); }

  function pickLanguage(saved) {
    var known = LANGUAGES.map(function (l) { return l[0]; });
    if (known.indexOf(saved) >= 0) return saved;
    var system = String(navigator.language || "").toLowerCase();
    if (/^zh-(tw|hk|mo|hant)/.test(system)) return "zht";
    var base = system.split("-")[0];
    return known.indexOf(base) >= 0 ? base : "en";
  }

  function applyLanguage() {
    ui.destLabel.setAttribute("data-i18n", s.downloadOnly ? "saveIn" : "destLabel");
    var tag = LANGUAGES.filter(function (l) { return l[0] === s.lang; })[0][2];
    document.documentElement.setAttribute("lang", tag);
    document.documentElement.setAttribute("dir", s.lang === "ar" ? "rtl" : "ltr");
    Array.prototype.forEach.call(document.querySelectorAll("[data-i18n]"), function (n) { n.textContent = t(n.getAttribute("data-i18n")); });
    Array.prototype.forEach.call(document.querySelectorAll("[data-i18n-placeholder]"), function (n) { n.setAttribute("placeholder", t(n.getAttribute("data-i18n-placeholder"))); });
    Array.prototype.forEach.call(document.querySelectorAll("[data-i18n-title]"), function (n) { n.setAttribute("title", t(n.getAttribute("data-i18n-title"))); });
    Array.prototype.forEach.call(document.querySelectorAll("[data-i18n-aria]"), function (n) { n.setAttribute("aria-label", t(n.getAttribute("data-i18n-aria"))); });
    var format = ui.cFormat.value || "mp4";
    while (ui.cFormat.firstChild) ui.cFormat.removeChild(ui.cFormat.firstChild);
    FORMATS.forEach(function (f) { addOption(ui.cFormat, f[0], f[1] + " (" + t(f[2]) + ")"); });
    ui.cFormat.value = format;
    setDest(s.dest);
    ui.cOut.textContent = s.outDir || t("sameFolder");
    ui.cGpuLabel.textContent = tr(s.gpu);
    renderTree();
    renderRows();
    s.files.forEach(paintRow);
    renderStatus();
    disarm();
    refreshControls();
    refreshSettingsTree();
    renderYoutubeLogin();
    renderRecent();
    renderDownloads();
    renderFreyLogin();
    fit();
  }

  // ---- destino: lista de caminhos recentes e subpastas da pasta escolhida

  function samePath(a, b) {
    var norm = function (p) { p = String(p || "").replace(/[\\/]+$/, ""); return s.mac ? p : p.toLowerCase(); };
    return !!a && norm(a) === norm(b);
  }

  function svgX() {
    var ns = "http://www.w3.org/2000/svg";
    var svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 10 10");
    svg.setAttribute("aria-hidden", "true");
    var path = document.createElementNS(ns, "path");
    path.setAttribute("d", "M2 2l6 6M8 2l-6 6");
    svg.appendChild(path);
    return svg;
  }

  function renderRecent() {
    var list = s.recent || [];
    ui.recentList.replaceChildren.apply(ui.recentList, list.map(function (place) {
      var li = document.createElement("li");
      li.className = "recent__item";
      li.setAttribute("role", "option");
      var current = samePath(place.path, s.dest);
      li.classList.toggle("is-current", current);
      li.setAttribute("aria-selected", String(current));
      var pick = document.createElement("button");
      pick.type = "button";
      pick.className = "recent__path";
      pick.title = place.path;
      var text = document.createElement("span");
      text.textContent = shownPath(place.path);
      pick.appendChild(text);
      pick.addEventListener("click", function () { chooseRecent(place.path); });
      var forget = document.createElement("button");
      forget.type = "button";
      forget.className = "recent__forget";
      forget.title = t("recentRemove");
      forget.setAttribute("aria-label", t("recentRemove"));
      forget.appendChild(svgX());
      forget.addEventListener("click", function () {
        invoke("forget_path", { path: place.path }).then(function (left) { s.recent = left; renderRecent(); fit(); });
      });
      li.appendChild(pick);
      li.appendChild(forget);
      return li;
    }));
    ui.recentEmpty.hidden = list.length > 0;
  }

  function toggleRecent(open) {
    s.recentOpen = open;
    ui.recent.hidden = !open;
    ui.destMore.setAttribute("aria-expanded", String(open));
    ui.viewProject.classList.toggle("is-recent-open", open);
    if (open) invoke("recent_paths").then(function (list) { s.recent = list; renderRecent(); fit(); });
    fit();
  }

  function chooseRecent(path) {
    setDest(path);
    clearConfirm();
    toggleRecent(false);
    focusProject();
  }

  // So baixar: as subpastas da pasta escolhida viram opcoes no menu de cada link.
  function refreshSubfolders() {
    var seq = ++s.subSeq;
    if (!s.dest) { applySubfolders([]); return; }
    invoke("list_subfolders", { path: s.dest }).then(function (list) { if (seq === s.subSeq) applySubfolders(list); });
  }

  function applySubfolders(list) {
    var same = s.subfolders && s.subfolders.join("\n") === list.join("\n");
    s.subfolders = list;
    ui.viewProject.classList.toggle("no-subfolders", !list.length);
    if (!same) renderRows();
  }

  function subfolderOptions(select, value) {
    while (select.firstChild) select.removeChild(select.firstChild);
    addOption(select, "", t("chosenFolder"));
    s.subfolders.forEach(function (name) { addOption(select, name, name); });
    select.value = value;
  }

  // ---- login do YouTube: desligado por padrao; so liga depois de aceitar o risco

  function youtubeSources() {
    // Chrome e Edge no Windows protegem o login contra outros programas: o yt-dlp nao consegue ler.
    // Brave usa a mesma base do Chrome e pode ter a mesma protecao: se nao der para ler, o app avisa.
    var list = [["", "ytLoginOff"], ["auto", "ytLoginAuto"], ["firefox", "ytLoginFirefox"], ["brave", "ytLoginBrave"]];
    if (s.mac) list.push(["safari", "ytLoginSafari"], ["chrome", "ytLoginChrome"]);
    list.push(["file", "ytLoginFile"]);
    return list;
  }

  function renderYoutubeLogin() {
    var y = s.yt || { source: "", file: "", risk: false };
    while (ui.sYtLogin.firstChild) ui.sYtLogin.removeChild(ui.sYtLogin.firstChild);
    youtubeSources().forEach(function (o) { addOption(ui.sYtLogin, o[0], t(o[1])); });
    ui.sYtRisk.checked = y.risk;
    ui.sYtLogin.value = y.risk ? y.source : "";
    ui.sYtLogin.disabled = !y.risk;
    ui.sYtCookies.hidden = !y.risk || y.source !== "file";
    ui.sYtCookiesPath.hidden = ui.sYtCookies.hidden;
    ui.sYtCookiesPath.textContent = y.file || t("ytLoginNoFile");
    if (ui.sFreyState) renderFreyLogin();
  }

  function saveYoutubeLogin() {
    invoke("set_youtube_login", { source: s.yt.risk ? s.yt.source : "", file: s.yt.file, accepted: s.yt.risk });
    renderYoutubeLogin();
    fit();
  }

  function addOption(select, value, label) {
    var option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    select.appendChild(option);
  }

  // ---- geral

  function fit() {
    ui.shell.style.minHeight = "";
    var top = ui.shell.getBoundingClientRect().top;
    var height = ui.shell.getBoundingClientRect().height;
    // A lista de recentes fica por cima do resto: o painel cresce ate ela caber.
    if (!ui.recent.hidden) {
      var need = ui.recent.getBoundingClientRect().bottom - top + 12;
      if (need > height) { height = need; ui.shell.style.minHeight = need + "px"; }
    }
    invoke("resize_panel", { height: Math.ceil(height) });
  }

  function renderStatus() {
    ui.status.textContent = tr(s.status);
    ui.status.className = "status" + (s.statusKind ? " is-" + s.statusKind : "");
  }

  function setStatus(message, kind) {
    s.status = message || "";
    s.statusKind = kind || "";
    renderStatus();
    fit();
  }

  function setDest(path) {
    s.dest = path || "";
    ui.dest.textContent = s.dest || t("pickFolder");
    ui.dest.title = s.dest;
    ui.dest.classList.toggle("is-empty", !s.dest);
    if (s.recentOpen) renderRecent();
    refreshSubfolders();
  }

  function refreshControls() {
    var converting = s.running && s.runningKind === "convert";
    if (s.tab === "convert") {
      var again = s.files.some(function (f) { return f.state === "fail" || f.state === "cancelled"; });
      ui.go.textContent = t(s.running ? "working" : again ? "tryAgain" : "convertBtn");
      ui.open.disabled = !s.convertOpen;
      ui.go.disabled = s.running;
    } else {
      var pending = pendingRows();
      var failed = s.rows.some(function (r) { return r.state === "fail" || r.state === "cancelled"; });
      // Baixando, os links novos entram na fila do que ja esta rodando.
      if (s.running && pending.length && !converting) ui.go.textContent = t("addToQueue", pending.length);
      else if (s.running) ui.go.textContent = t("working");
      else if (s.confirmNext) ui.go.textContent = t("completeExisting");
      else if (failed) ui.go.textContent = t("tryAgain");
      else if (s.downloadOnly) ui.go.textContent = t("downloadBtn");
      else ui.go.textContent = t(s.rows.length ? "createAndDownload" : "createProject");
      ui.open.disabled = !s.projectOpen;
      ui.go.disabled = converting || (s.running && !pending.length) || (s.broken && !s.downloadOnly);
    }
    ui.cancel.hidden = !s.running;
    ui.cancel.textContent = t(s.runningKind === "download" ? "stopAll" : "cancel");
    ui.actions.hidden = s.tab === "settings" && !s.running;
    ui.go.hidden = s.tab === "downloads";
    ui.open.hidden = s.tab === "downloads";
    ui.clearDone.hidden = s.tab !== "downloads";
    // Modelos com erro so travam o que depende deles: baixar continua possivel.
    [ui.name, ui.model].forEach(function (n) { n.disabled = s.broken; });
    ui.createFolders.disabled = s.broken && !s.downloadOnly;
    [ui.cFormat, ui.cMode, ui.cQuality, ui.cOut, ui.cAdd].forEach(function (n) { n.disabled = s.running; });
    if (!s.running) syncModeControl();
    [ui.sModel, ui.sNew, ui.sDuplicate, ui.sName, ui.sRoot, ui.sAddFolder, ui.sOpenAtEnd].forEach(function (n) { n.disabled = s.broken; });
    ui.sDelete.disabled = s.broken || s.models.length <= 1;
  }

  // Criar pastas pede o nome so se o modelo tiver onde usar; So baixar esconde o que e da estrutura.
  function applyMode() {
    ui.createFolders.checked = !s.downloadOnly;
    ui.createFoldersHint.textContent = t(s.downloadOnly ? "createFoldersOff" : "createFoldersOn");
    ui.queryRow.hidden = s.downloadOnly || !s.needsName;
    ui.fieldModel.hidden = s.downloadOnly;
    ui.tree.hidden = s.downloadOnly;
    ui.viewProject.classList.toggle("download-only", s.downloadOnly);
    ui.destLabel.setAttribute("data-i18n", s.downloadOnly ? "saveIn" : "destLabel");
    ui.destLabel.textContent = t(s.downloadOnly ? "saveIn" : "destLabel");
    refreshControls();
    fit();
  }

  function focusProject() {
    if (s.tab !== "project") return;
    if (!s.downloadOnly && s.needsName && !s.broken) { ui.name.focus(); ui.name.select(); }
    else ui.links.focus();
  }

  function setDownloadOnly(value) {
    s.downloadOnly = value;
    s.confirmNext = false;
    if (s.status === "err.nameNeeded" || s.status === "err.noLinks") setStatus("");
    renderRows();
    applyMode();
    focusProject();
    invoke("set_download_only", { downloadOnly: value });
  }

  function syncModeControl() {
    ui.cMode.disabled = ["mp4", "mov", "mkv"].indexOf(ui.cFormat.value) < 0;
  }

  function selectTab(tab) {
    s.tab = tab;
    [["project", ui.tabProject, ui.viewProject], ["downloads", ui.tabDownloads, ui.viewDownloads], ["convert", ui.tabConvert, ui.viewConvert], ["settings", ui.tabSettings, ui.viewSettings]].forEach(function (x) {
      x[1].setAttribute("aria-selected", String(x[0] === tab));
      x[2].hidden = x[0] !== tab;
    });
    disarm();
    refreshControls();
    if (tab === "convert") checkTools();
    if (tab === "settings") refreshSettingsTree();
    if (tab === "downloads") renderDownloads();
    if ((tab !== "settings" && s.status === "saved") || (tab !== "project" && s.status === "linkDetected")) setStatus("");
    fit();
  }

  function checkTools() {
    if (s.toolsChecked) return;
    s.toolsChecked = true;
    invoke("convert_tools").then(function (tools) {
      if (!tools.ffmpeg) {
        s.gpu = "noFfmpeg";
        setStatus("err.ffmpegMissing", "error");
      } else if (tools.gpu) {
        ui.cGpu.disabled = false;
        ui.cGpu.checked = true;
        s.gpu = c("useGpu", tools.gpu);
      } else {
        s.gpu = "noGpu";
      }
      ui.cGpuLabel.textContent = tr(s.gpu);
      fit();
    });
  }

  function makeRemoveButton(onClick) {
    var remove = document.createElement("button");
    remove.type = "button";
    remove.setAttribute("data-i18n-title", "remove");
    remove.setAttribute("data-i18n-aria", "remove");
    remove.title = t("remove");
    remove.setAttribute("aria-label", t("remove"));
    var icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    icon.setAttribute("viewBox", "0 0 12 12");
    var path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M2 2l8 8M10 2l-8 8");
    icon.appendChild(path);
    remove.appendChild(icon);
    remove.addEventListener("click", onClick);
    return remove;
  }

  function makeRow(list, kindKey, title, selects, onRemove) {
    var li = document.createElement("li");
    li.className = "row";
    var kind = document.createElement("span");
    kind.className = "row__kind";
    kind.setAttribute("data-i18n", kindKey);
    kind.textContent = t(kindKey);
    var main = document.createElement("div");
    main.className = "row__main";
    var link = document.createElement("div");
    link.className = "row__link";
    link.dir = "ltr";
    link.textContent = title;
    link.title = title;
    var state = document.createElement("div");
    state.className = "row__state";
    main.appendChild(link);
    main.appendChild(state);
    var remove = makeRemoveButton(function () { onRemove(); li.remove(); refreshControls(); fit(); });
    li.appendChild(kind);
    li.appendChild(main);
    selects.forEach(function (select) { li.appendChild(select); });
    li.appendChild(remove);
    list.appendChild(li);
    return { li: li, state: state, selects: selects, folder: selects[selects.length - 1], remove: remove };
  }

  // row.msg: lista de mensagens, unidas por " · " no idioma atual.
  function paintRow(row) {
    var text = row.msg.map(tr).join(" · ");
    row.node.state.textContent = text;
    row.node.state.title = text;
    var cls = { ok: "is-ok", running: "is-running", fail: "is-fail", warning: "is-warning" }[row.state];
    row.node.state.className = "row__state" + (cls ? " " + cls : "");
    row.node.selects.forEach(function (select) { select.disabled = s.running || row.state === "ok" || row.state === "warning"; });
    row.node.remove.disabled = s.running;
  }

  // ---- aba Projeto: cada link vira um cartao com capa, titulo e as escolhas dele

  var SITE_TAG = { youtube: "YT", instagram: "IG", tiktok: "TT", x: "X", file: "GD", folder: "GD" };
  var SITE_NAME = { youtube: "YouTube", instagram: "Instagram" };
  var BROWSER_NAME = { firefox: "Firefox", brave: "Brave", opera: "Opera", vivaldi: "Vivaldi", chrome: "Chrome", edge: "Edge", chromium: "Chromium", safari: "Safari", file: "cookies.txt", frey: "Frey" };
  var LOGIN_OFF = ["err.ytLoginOff", "err.igLoginOff"];
  var LOGIN_MISSING = ["err.ytCookiesRead", "err.ytLoginRefused", "err.igLoginRefused"];

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  // "12:48" ou "1:02:05".
  function clock(seconds) {
    if (!(seconds > 0)) return seconds === 0 ? "0:00" : "";
    var total = Math.round(seconds), h = Math.floor(total / 3600), m = Math.floor(total / 60) % 60, sec = total % 60;
    var two = function (n) { return (n < 10 ? "0" : "") + n; };
    return h ? h + ":" + two(m) + ":" + two(sec) : m + ":" + two(sec);
  }

  // "2:50", "170" ou "1:02:05" para segundos; NaN quando nao entende.
  function parseClock(text) {
    var parts = String(text).trim().split(":").map(Number);
    if (!parts.length || parts.some(function (n) { return isNaN(n) || n < 0; })) return NaN;
    return parts.reduce(function (acc, n) { return acc * 60 + n; }, 0);
  }

  // Capas chegam do Rust como data: (a politica do app nao libera imagens de outros sites).
  var thumbCache = {};
  function loadThumb(url, img) {
    if (!url) return;
    if (!thumbCache[url]) thumbCache[url] = invoke("thumb_data", { url: url }).then(function (d) { return d; }, function () { return ""; });
    thumbCache[url].then(function (data) { if (data && img.isConnected !== false) { img.src = data; img.hidden = false; } });
  }

  function makeThumb(kind, thumb, time) {
    var box = el("div", "thumb");
    var img = el("img");
    img.alt = "";
    img.hidden = true;
    box.appendChild(img);
    var tag = el("span", "thumb__site " + kind, SITE_TAG[kind] || "");
    box.appendChild(tag);
    var t2 = el("span", "thumb__time");
    t2.hidden = true;
    box.appendChild(t2);
    var api = {
      box: box,
      set: function (url, timeText) {
        if (url && api.url !== url) { api.url = url; loadThumb(url, img); }
        t2.textContent = timeText || "";
        t2.hidden = !timeText;
      }
    };
    api.set(thumb, time);
    return api;
  }

  function folderOptions(select, value) {
    while (select.firstChild) select.removeChild(select.firstChild);
    addOption(select, "", t("projectRoot"));
    s.folders.forEach(function (folder) { addOption(select, folder, shownPath(folder)); });
    select.value = value;
  }

  // 2K e 4K so aparecem quando o video tem (ou quando ainda nao se sabe); 4K para edicao so no YouTube.
  function qualityOptions(select, value, row) {
    var height = row.probe && row.probe.meta && row.probe.meta.height;
    while (select.firstChild) select.removeChild(select.firstChild);
    QUALITIES.forEach(function (q) {
      if ((q[0] === "edit4k" || q[0] === "2160" || q[0] === "1440") && row.kind !== "youtube") return;
      if (q[0] === "2160" && height && height < 2160) return;
      if (q[0] === "1440" && height && height < 1440) return;
      addOption(select, q[0], t(q[1], q[2]));
    });
    select.title = t("qualityHint");
    select.setAttribute("aria-label", t("videoQuality"));
    select.value = value;
    if (select.value !== value) { select.value = "1080"; row.quality = "1080"; }
  }

  function lastQuality(kind) {
    var q = localStorage.getItem("fp.quality") || "1080";
    if (kind !== "youtube" && (q === "edit4k" || q === "2160" || q === "1440")) return "1080";
    return q;
  }

  function chip(label, onClick, title) {
    var b = el("button", "chipbtn", label);
    b.type = "button";
    if (title) { b.title = title; b.setAttribute("aria-label", title); }
    b.addEventListener("click", onClick);
    return b;
  }

  function removeRow(row) {
    if (row.state === "queued" || row.state === "running") {
      (row.parts || [row]).forEach(function (p) { invoke("cancel_item", { id: p.id }); });
      return;
    }
    s.rows = s.rows.filter(function (r) { return r !== row; });
    row.node.li.remove();
    refreshControls();
    fit();
  }

  function buildCard(row) {
    var li = el("li", "card");
    var thumb = makeThumb(row.kind, "", "");
    var meta = el("div", "card__meta");
    var title = el("div", "card__title", row.url);
    title.title = row.url;
    var sub = el("div", "card__sub");
    var ctrls = el("div", "card__ctrls");
    var quality = null;
    if (VIDEO_KINDS[row.kind]) {
      quality = document.createElement("select");
      quality.className = "row__quality";
      quality.addEventListener("change", function () { row.quality = quality.value; localStorage.setItem("fp.quality", quality.value); });
      ctrls.appendChild(quality);
    }
    var folder = document.createElement("select");
    folder.className = "card__folder";
    folder.setAttribute("aria-label", t("downloadTo"));
    folder.addEventListener("change", function () {
      if (s.downloadOnly) row.sub = folder.value;
      else { row.folder = folder.value; row.folderChosen = true; }
    });
    ctrls.appendChild(folder);
    var trimBtn = chip("", function () { row.trimOpen = !row.trimOpen; paintCard(row); fit(); }, t("trim"));
    trimBtn.appendChild(scissors());
    trimBtn.appendChild(el("span"));
    var subsBtn = chip("CC", function () { row.subs = !row.subs; paintCard(row); }, t("subs"));
    ctrls.appendChild(trimBtn);
    ctrls.appendChild(subsBtn);
    var remove = makeRemoveButton(function () { removeRow(row); });
    remove.className = "chipbtn card__remove";
    ctrls.appendChild(remove);
    meta.appendChild(title);
    meta.appendChild(sub);
    meta.appendChild(ctrls);
    var bar = el("div", "card__bar");
    bar.appendChild(el("i"));
    bar.hidden = true;
    var extra = el("div", "card__extra");
    extra.hidden = true;
    li.appendChild(thumb.box);
    li.appendChild(meta);
    li.appendChild(bar);
    li.appendChild(extra);
    ui.rows.appendChild(li);
    row.node = { li: li, thumb: thumb, title: title, sub: sub, quality: quality, folder: folder, trim: trimBtn, subs: subsBtn, remove: remove, bar: bar, extra: extra };
  }

  function scissors() {
    var ns = "http://www.w3.org/2000/svg";
    var svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 12 12");
    var p = document.createElementNS(ns, "path");
    p.setAttribute("d", "M3 3.2a1.4 1.4 0 1 1 0 .01M3 8.8a1.4 1.4 0 1 1 0 .01M4.2 4.1 10.5 9M4.2 7.9 10.5 3");
    svg.appendChild(p);
    return svg;
  }

  function hasTrim(row) { return row.start != null && row.end != null && row.end > row.start; }

  // Texto da linha de baixo do cartao, conforme o momento.
  function cardSub(row) {
    if (row.state !== "pending" || row.msg.length) {
      if (row.msg.length) return { text: row.msg.map(tr).join(" · "), cls: { ok: "is-ok", running: "is-running", fail: "is-fail", warning: "is-warning" }[row.state] || "" };
    }
    var p = row.probe;
    if (!p) return { text: VIDEO_KINDS[row.kind] ? t("probing") : t(row.kind === "folder" ? "kindFolder" : "kindFile") + " · Google Drive", cls: "" };
    if (p.status === "loading") return { text: t("probing"), cls: "" };
    if (p.error && LOGIN_OFF.concat(LOGIN_MISSING).indexOf(p.error) < 0) return { text: tr(p.error), cls: "is-fail" };
    var m = p.meta;
    if (!m) return { text: "", cls: "" };
    var parts = [];
    if (m.entries.length) parts.push(t("playlistCount", row.picked.length, m.entries.length));
    if (m.uploader) parts.push(m.uploader);
    if (m.duration) parts.push(clock(m.duration));
    if (m.height >= 1440) parts.push(t("maxRes", m.height >= 2160 ? "4K" : "2K"));
    return { text: parts.join(" · "), cls: "" };
  }

  function paintCard(row) {
    if (!row.node) buildCard(row);
    var n = row.node, p = row.probe, m = p && p.meta;
    var busy = row.state === "queued" || row.state === "running";
    var done = row.state === "ok" || row.state === "warning";
    n.li.classList.toggle("is-ok", done);
    n.li.classList.toggle("is-fail", row.state === "fail");
    var title = m && m.title ? (m.entries.length ? t("playlistLabel", m.title) : m.title) : row.url;
    n.title.textContent = title;
    n.title.title = row.url;
    var badge = m ? (m.entries.length ? t("videosCount", m.entries.length) : (m.duration ? clock(m.duration) : "")) : "";
    n.thumb.set(m && (m.thumbnail || (m.entries[0] && m.entries[0].thumbnail)), badge);
    var sub = cardSub(row);
    n.sub.textContent = sub.text;
    n.sub.title = sub.text;
    n.sub.className = "card__sub" + (sub.cls ? " " + sub.cls : "");
    if (n.quality) { qualityOptions(n.quality, row.quality, row); n.quality.disabled = busy || done; }
    n.folder.disabled = busy || done;
    var canTrim = !!(m && m.duration && !m.entries.length && VIDEO_KINDS[row.kind]);
    n.trim.hidden = !canTrim;
    n.trim.disabled = busy || done;
    n.trim.setAttribute("aria-pressed", String(hasTrim(row) || !!row.trimOpen));
    n.trim.lastChild.textContent = hasTrim(row) ? clock(row.start) + "–" + clock(row.end) : "";
    n.subs.hidden = row.kind !== "youtube";
    n.subs.disabled = busy || done;
    n.subs.setAttribute("aria-pressed", String(!!row.subs));
    n.remove.title = t(busy ? "stopItem" : "remove");
    n.remove.setAttribute("aria-label", n.remove.title);
    n.bar.hidden = row.state !== "running";
    if (row.percent != null) n.bar.firstChild.style.width = Math.min(100, row.percent) + "%";
    paintExtra(row);
  }

  // Area de baixo do cartao: corte, lista da playlist ou o login.
  function paintExtra(row) {
    var box = row.node.extra, p = row.probe;
    var key = extraKey(row);
    if (box.dataset.key === key) { syncTrim(row); return; }
    box.dataset.key = key;
    while (box.firstChild) box.removeChild(box.firstChild);
    box.className = "card__extra";
    box.hidden = !key;
    if (!key) return;
    if (key.indexOf("trim") === 0) buildTrim(row, box);
    else if (key.indexOf("playlist") === 0) buildPlaylist(row, box);
    else if (key.indexOf("found") === 0) buildLoginFound(row, box);
    else if (key.indexOf("login") === 0) buildLoginMissing(row, box);
    if (p && p.error && key.indexOf("login") !== 0) box.hidden = true;
  }

  function extraKey(row) {
    var p = row.probe, m = p && p.meta;
    if (row.state !== "pending" && row.state !== "fail" && row.state !== "cancelled") return "";
    if (p && p.error && (LOGIN_OFF.indexOf(p.error) >= 0 || LOGIN_MISSING.indexOf(p.error) >= 0)) return "login:" + p.error + ":" + s.lang;
    if (row.trimOpen && m && m.duration) return "trim:" + s.lang;
    if (m && m.entries.length) return "playlist:" + s.lang;
    if (p && p.loginVia) return "found:" + p.loginVia + ":" + s.lang;
    return "";
  }

  function buildTrim(row, box) {
    var m = row.probe.meta, dur = Math.round(m.duration);
    if (!hasTrim(row)) { row.start = 0; row.end = dur; }
    box.classList.add("card__extra--gold");
    var top = el("div", "extra__top");
    top.appendChild(el("span", "", t("trimOnly")));
    var len = el("span");
    top.appendChild(len);
    var range = el("div", "range2");
    var fill = el("div", "range2__fill");
    var a = document.createElement("input"), b = document.createElement("input");
    [a, b].forEach(function (r) { r.type = "range"; r.min = "0"; r.max = String(dur); r.step = "1"; });
    a.setAttribute("aria-label", t("trimStart"));
    b.setAttribute("aria-label", t("trimEnd"));
    range.appendChild(fill);
    range.appendChild(a);
    range.appendChild(b);
    var times = el("div", "trim-times");
    var ia = document.createElement("input"), ib = document.createElement("input");
    ia.setAttribute("aria-label", t("trimStart"));
    ib.setAttribute("aria-label", t("trimEnd"));
    times.appendChild(el("span", "", t("trimStart")));
    times.appendChild(ia);
    times.appendChild(el("span", "", t("trimEnd")));
    times.appendChild(ib);
    if (m.chapters.length) {
      var sel = document.createElement("select");
      addOption(sel, "", t("trimChapters"));
      m.chapters.forEach(function (ch, i) { addOption(sel, String(i), clock(ch.start) + " " + ch.title); });
      sel.addEventListener("change", function () {
        var ch = m.chapters[Number(sel.value)];
        if (ch) { row.start = Math.round(ch.start); row.end = Math.round(ch.end); syncTrim(row); paintCard(row); }
      });
      times.appendChild(sel);
    }
    var whole = el("button", "link-btn", t("trimWhole"));
    whole.type = "button";
    whole.addEventListener("click", function () { row.start = null; row.end = null; row.trimOpen = false; paintCard(row); fit(); });
    times.appendChild(whole);
    box.appendChild(top);
    box.appendChild(range);
    box.appendChild(times);
    row.trimUi = { a: a, b: b, ia: ia, ib: ib, fill: fill, len: len, dur: dur };
    var fromRange = function () {
      var x = Number(a.value), y = Number(b.value);
      row.start = Math.min(x, y); row.end = Math.max(x, y);
      syncTrim(row);
      row.node.trim.lastChild.textContent = clock(row.start) + "–" + clock(row.end);
    };
    a.addEventListener("input", fromRange);
    b.addEventListener("input", fromRange);
    var fromText = function () {
      var x = parseClock(ia.value), y = parseClock(ib.value);
      if (!isNaN(x) && !isNaN(y) && y > x) { row.start = Math.min(x, dur); row.end = Math.min(y, dur); }
      syncTrim(row);
      paintCard(row);
    };
    ia.addEventListener("change", fromText);
    ib.addEventListener("change", fromText);
    syncTrim(row);
  }

  function syncTrim(row) {
    var u = row.trimUi;
    if (!u || !row.node.extra.contains(u.a) || !hasTrim(row)) return;
    u.a.value = String(row.start);
    u.b.value = String(row.end);
    if (document.activeElement !== u.ia) u.ia.value = clock(row.start);
    if (document.activeElement !== u.ib) u.ib.value = clock(row.end);
    u.fill.style.insetInlineStart = (row.start / u.dur * 100) + "%";
    u.fill.style.width = ((row.end - row.start) / u.dur * 100) + "%";
    u.len.textContent = t("trimOf", clock(row.end - row.start), clock(u.dur));
  }

  function buildPlaylist(row, box) {
    var m = row.probe.meta;
    var top = el("div", "extra__top");
    var count = el("span", "", t("playlistCount", row.picked.length, m.entries.length));
    var toggle = el("button", "link-btn");
    toggle.type = "button";
    top.appendChild(count);
    top.appendChild(toggle);
    var list = el("ul", "pl-list");
    var boxes = [];
    var update = function () {
      count.textContent = t("playlistCount", row.picked.length, m.entries.length);
      toggle.textContent = t(row.picked.length === m.entries.length ? "markNone" : "markAll");
      var sub = cardSub(row);
      row.node.sub.textContent = sub.text;
    };
    m.entries.forEach(function (entry) {
      var li = el("li"), label = el("label"), cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = row.picked.indexOf(entry.url) >= 0;
      cb.addEventListener("change", function () {
        row.picked = m.entries.filter(function (e, i) { return boxes[i].checked; }).map(function (e) { return e.url; });
        update();
      });
      boxes.push(cb);
      label.appendChild(cb);
      label.appendChild(el("span", "pl__title", entry.title || entry.url));
      label.appendChild(el("span", "pl__time", clock(entry.duration)));
      li.appendChild(label);
      list.appendChild(li);
    });
    toggle.addEventListener("click", function () {
      var all = row.picked.length !== m.entries.length;
      boxes.forEach(function (cb) { cb.checked = all; });
      row.picked = all ? m.entries.map(function (e) { return e.url; }) : [];
      update();
    });
    box.appendChild(top);
    box.appendChild(list);
    update();
  }

  function buildLoginFound(row, box) {
    box.classList.add("card__extra--gold");
    var line = el("div", "login-line");
    line.appendChild(el("span", "login-dot"));
    line.appendChild(el("b", "", t("loginFound", SITE_NAME[row.probe.site] || "", BROWSER_NAME[row.probe.loginVia] || row.probe.loginVia)));
    box.appendChild(line);
  }

  function buildLoginMissing(row, box) {
    var p = row.probe, site = SITE_NAME[p.site] || "";
    var off = LOGIN_OFF.indexOf(p.error) >= 0;
    var line = el("div", "login-line");
    line.appendChild(el("span", "login-dot is-off"));
    line.appendChild(el("b", "", off ? tr(p.error) : t("loginNotFound", site)));
    box.appendChild(line);
    if (!off && p.browsers.length) {
      var list = el("div", "browsers");
      p.browsers.forEach(function (b) { list.appendChild(el("span", "is-no", BROWSER_NAME[b] || b)); });
      box.appendChild(list);
      if (!s.mac && p.browsers.some(function (b) { return b === "chrome" || b === "edge"; })) box.appendChild(el("span", "hint-inline", t("loginProtected")));
    }
    var buttons = el("div", "extra__buttons");
    if (!off) {
      var frey = el("button", "btn btn--primary", t("loginWithFrey"));
      frey.type = "button";
      frey.addEventListener("click", function () { invoke("open_login", { site: p.site }).then(null, function (e) { setStatus(e, "error"); }); });
      buttons.appendChild(frey);
    }
    var settings = el("button", "btn", t("openSettings"));
    settings.type = "button";
    settings.addEventListener("click", function () { selectTab("settings"); });
    var skip = el("button", "btn", t("skip"));
    skip.type = "button";
    skip.addEventListener("click", function () { removeRow(row); });
    buttons.appendChild(settings);
    buttons.appendChild(skip);
    box.appendChild(buttons);
  }

  function renderLink(row) { paintCard(row); }

  function renderRows() {
    document.getElementById("viewProject").classList.toggle("download-only", s.downloadOnly);
    s.rows.forEach(function (row) {
      // Link que o usuario nao mudou acompanha a pasta de destino do modelo.
      if (!row.folderChosen && row.state === "pending") row.folder = s.defaultFolder;
      if (s.folders.indexOf(row.folder) < 0 && row.folder !== "") row.folder = s.defaultFolder;
      paintCard(row);
      if (s.downloadOnly) {
        if (s.subfolders.indexOf(row.sub) < 0) row.sub = "";
        subfolderOptions(row.node.folder, row.sub || "");
      } else folderOptions(row.node.folder, row.folder);
    });
    fit();
  }

  // Os modelos guardam subpastas com "\"; no Mac o painel mostra "/".
  function shownPath(rel) { return s.mac ? String(rel).replace(/\\/g, "/") : rel; }

  function fillTree(target, root, folders) {
    while (target.firstChild) target.removeChild(target.firstChild);
    var b = document.createElement("b");
    b.textContent = shownPath((root || t("chosenFolder")) + "\\") + "  ";
    target.appendChild(b);
    var tops = folders.filter(function (f) { return f.indexOf("\\") < 0; });
    target.appendChild(document.createTextNode(tops.join("  ·  ")));
  }

  function renderTree() { fillTree(ui.tree, s.root, s.folders); }

  function refreshPreview() {
    var seq = ++s.requestSeq;
    if (!ui.model.value) return;
    // Sem nome ainda, a previa mostra um exemplo; so a previa, nunca o que sera criado.
    invoke("preview", { model: ui.model.value, name: ui.name.value.trim() || t("defaultName") }).then(function (p) {
      if (seq !== s.requestSeq) return;
      s.folders = p.folders;
      s.defaultFolder = p.defaultFolder;
      s.root = p.root;
      s.needsName = p.needsName;
      renderTree();
      renderRows();
      applyMode();
    }, function (error) { setStatus(error, "error"); });
  }

  // Previa do link: capa, titulo, duracao, capitulos, playlist e o login, quando o site pede.
  function probe(row) {
    if (!VIDEO_KINDS[row.kind]) return;
    row.probe = { status: "loading" };
    paintCard(row);
    invoke("probe_link", { url: row.url }).then(function (answer) {
      if (s.rows.indexOf(row) < 0) return;
      row.probe = { status: answer.meta ? "ok" : "error", meta: answer.meta, error: answer.error, site: answer.site, loginVia: answer.loginVia, browsers: answer.browsers || [] };
      if (answer.meta && answer.meta.entries.length && !row.picked.length) row.picked = answer.meta.entries.map(function (e) { return e.url; });
      paintCard(row);
      fit();
    }, function () { row.probe = { status: "error", error: "err.ytFailed", browsers: [] }; paintCard(row); });
  }

  function addLinks(text) {
    return invoke("parse_links", { text: text }).then(function (links) {
      var added = [];
      links.forEach(function (link) {
        if (s.rows.some(function (r) { return r.url === link.url && r.state !== "ok" && r.state !== "warning"; })) return;
        var row = { id: s.nextId++, url: link.url, kind: link.kind, folder: s.defaultFolder, quality: lastQuality(link.kind), state: "pending", msg: [], node: null, picked: [] };
        s.rows.push(row);
        added.push(row);
      });
      renderRows();
      added.forEach(probe);
      refreshControls();
      if (added.length && s.quick) quickStart();
      return links.length;
    });
  }

  // Modo rapido: colou, baixou. Com a estrutura ligada, so falta o nome do projeto.
  function quickStart() {
    if (!s.downloadOnly && s.needsName && !ui.name.value.trim()) { setStatus("quickNeedsName", "warning"); ui.name.focus(); return; }
    if (s.runningKind === "convert") return;
    startProject(false);
  }

  // Marca os itens antes de chamar: os eventos do Rust podem chegar antes da resposta do invoke.
  function beginRun(items, paint, kind) {
    var saved = items.map(function (r) { return { row: r, state: r.state, msg: r.msg }; });
    var wasRunning = s.running;
    s.running = true;
    s.runningKind = kind || "convert";
    items.forEach(function (r) { r.state = "queued"; r.msg = ["queued"]; paint(r); });
    if (!wasRunning) {
      ui.track.hidden = false;
      ui.bar.style.width = "0";
      setStatus("preparing");
    }
    refreshControls();
    return function () {
      if (!wasRunning) { s.running = false; s.runningKind = ""; ui.track.hidden = true; setStatus(""); }
      saved.forEach(function (o) { o.row.state = o.state; o.row.msg = o.msg; paint(o.row); });
      refreshControls();
    };
  }

  function pendingRows() {
    return s.rows.filter(function (r) { return r.state === "pending" || r.state === "fail" || r.state === "cancelled"; });
  }

  // Cada link vira um item do Rust; uma playlist vira um item por video marcado.
  function itemsFor(rows) {
    var items = [];
    rows.forEach(function (r) {
      var base = { kind: r.kind, folder: s.downloadOnly ? (r.sub || "") : r.folder, quality: r.quality, subs: !!r.subs };
      var m = r.probe && r.probe.meta;
      if (m && m.entries.length) {
        r.parts = m.entries.filter(function (e) { return r.picked.indexOf(e.url) >= 0; }).map(function (e) {
          var part = { id: s.nextId++, url: e.url, title: e.title, thumb: e.thumbnail, state: "queued" };
          s.partOf[part.id] = r;
          items.push(Object.assign({ id: part.id, url: e.url, title: e.title, thumb: e.thumbnail || "" }, base));
          return part;
        });
        return;
      }
      r.parts = null;
      var item = Object.assign({ id: r.id, url: r.url, title: (m && m.title) || "", thumb: (m && m.thumbnail) || "" }, base);
      if (hasTrim(r) && !(r.start === 0 && m && r.end >= Math.round(m.duration))) { item.start = r.start; item.end = r.end; }
      items.push(item);
    });
    return items;
  }

  function startProject(confirmExisting) {
    var pending = pendingRows().filter(function (r) { return !(r.probe && r.probe.meta && r.probe.meta.entries.length && !r.picked.length); });
    var items = itemsFor(pending);
    var undo = beginRun(pending, renderLink, "download");
    items.forEach(function (item) { trackLive(item, "queued"); });
    invoke("start_job", { request: {
      model: ui.model.value, name: ui.name.value, dest: s.dest, confirmExisting: confirmExisting, convertTo: ui.convertTo.value, downloadOnly: s.downloadOnly,
      items: items
    } }).then(function (answer) {
      if (answer.status === "needs_confirm") {
        items.forEach(function (item) { s.live.delete(item.id); });
        undo();
        s.confirmNext = true;
        setStatus(c("existingConfirm", answer.root), "warning");
        refreshControls();
        renderDownloads();
        return;
      }
      s.projectOpen = true;
      refreshControls();
      if (answer.status === "created") { undo(); setStatus(c("createdAt", answer.base)); }
      if (answer.status === "queued") setStatus("queuedBatch");
    }, function (error) {
      items.forEach(function (item) { s.live.delete(item.id); });
      undo();
      renderDownloads();
      setStatus(error, "error");
      if (error === "err.nameNeeded") ui.name.focus();
    });
  }

  function formatSize(bytes) {
    if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(2) + " GB";
    if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + " MB";
    return Math.round(bytes / 1024) + " KB";
  }

  function find(list, id) { return list.filter(function (r) { return r.id === id; })[0]; }

  function setBar(percent) {
    ui.bar.classList.remove("is-indeterminate");
    ui.bar.style.width = Math.min(100, percent) + "%";
  }

  // Velocidade media dos ultimos segundos e tempo que falta.
  function speedOf(live, p) {
    var now = Date.now();
    if (live.lastAt && p.done >= live.lastDone && now > live.lastAt) {
      var bps = (p.done - live.lastDone) * 1000 / (now - live.lastAt);
      live.bps = live.bps ? live.bps * 0.7 + bps * 0.3 : bps;
    }
    if (!live.lastAt || now - live.lastAt > 400) { live.lastAt = now; live.lastDone = p.done; }
    if (!live.bps || !p.total) return "";
    return t("speedEta", formatSize(live.bps), clock((p.total - p.done) / live.bps));
  }

  // Playlist: o cartao mostra quantos ja foram e o video da vez.
  function paintPlaylist(row) {
    var parts = row.parts || [];
    var ok = parts.filter(function (p) { return p.state === "ok" || p.state === "warning"; }).length;
    var failed = parts.filter(function (p) { return p.state === "fail"; }).length;
    var open = parts.filter(function (p) { return p.state === "queued" || p.state === "running"; }).length;
    var current = parts.filter(function (p) { return p.state === "running"; })[0];
    row.state = open ? "running" : failed ? "fail" : "ok";
    row.msg = [c("playlistDone", ok, parts.length)];
    if (failed) row.msg.push(c("playlistFailed", failed));
    if (current) row.msg.push(current.title || current.url);
    paintCard(row);
  }

  function onJob(ev) {
    var row = ev.id != null ? (find(s.rows, ev.id) || s.partOf[ev.id]) : null;
    var part = row && row.parts ? find(row.parts, ev.id) : null;
    var live = ev.id != null ? s.live.get(ev.id) : null;
    switch (ev.event) {
      case "start":
        if (live) { live.state = "running"; live.msg = t("downloading"); }
        if (part) { part.state = "running"; paintPlaylist(row); }
        else if (row) { row.state = "running"; row.msg = ["downloading"]; row.percent = 0; renderLink(row); }
        ui.bar.style.width = "0";
        setStatus("connecting");
        break;
      case "progress": {
        var p = ev.progress;
        var parts = [p.file];
        if (p.filesTotal > 1) parts.unshift(t("fileOf", p.filesDone + 1, p.filesTotal));
        var speed = live ? speedOf(live, p) : "";
        if (p.total) {
          var percent = Math.floor(p.done * 100 / p.total);
          parts.push(t("sizeOf", percent, formatSize(p.done), formatSize(p.total)));
          setBar(percent);
          if (row && !part) { row.percent = percent; row.msg = [c("sizeOf", percent, formatSize(p.done), formatSize(p.total))]; if (speed) row.msg.push(speed); renderLink(row); }
        } else {
          parts.push(formatSize(p.done));
          ui.bar.classList.add("is-indeterminate");
        }
        if (speed) parts.push(speed);
        if (live) { live.done = p.done; live.total = p.total; live.msg = parts.slice(1).join(" · "); }
        ui.status.textContent = parts.join(" · ");
        break;
      }
      case "converting":
        if (live) live.msg = t("convertingFile", ev.file, Math.floor(ev.percent));
        if (row && !part) { row.msg = ["converting"]; renderLink(row); }
        setBar(ev.percent);
        ui.status.textContent = t("convertingFile", ev.file, Math.floor(ev.percent));
        break;
      case "ok":
        s.live.delete(ev.id);
        if (part) { part.state = ev.convertError ? "warning" : "ok"; paintPlaylist(row); }
        else if (row) {
          row.state = "ok";
          row.msg = [c("doneFiles", ev.downloaded)];
          if (ev.skipped) row.msg.push(c("skippedNote", ev.skipped));
          if (ev.converted) row.msg.push(c("convertedNote", ev.converted));
          if (ev.convertError) {
            row.state = "warning";
            row.msg.push(c("convertFailedNote", ev.convertError));
          }
          renderLink(row);
        }
        fit();
        break;
      case "fail":
        s.live.delete(ev.id);
        if (part) { part.state = "fail"; paintPlaylist(row); }
        else if (row) { row.state = "fail"; row.msg = [ev.message]; renderLink(row); }
        fit();
        break;
      case "cancelled":
        s.live.delete(ev.id);
        if (part) { part.state = "cancelled"; paintPlaylist(row); }
        else if (row) { row.state = "cancelled"; row.msg = ["cancelledKept"]; renderLink(row); }
        break;
      case "done":
        if (!ev.more) {
          s.running = false;
          s.runningKind = "";
          ui.track.hidden = true;
          s.live.clear();
        }
        refreshControls();
        s.rows.forEach(renderLink);
        if (ev.cancelled) setStatus(c("downloadCancelled", ev.ok), "warning");
        else if (ev.failed) setStatus(c("downloadSomeFailed", ev.ok, ev.failed), "error");
        else setStatus(c("downloadDone", ev.ok, ev.base));
        break;
    }
    renderDownloadsSoon();
  }

  // ---- aba Baixados: o que esta baixando agora e o historico

  function trackLive(item, state) {
    s.live.set(item.id, { id: item.id, url: item.url, kind: item.kind, title: item.title || item.url, thumb: item.thumb || "", state: state, msg: "" });
  }

  var FILTERS = ["all", "running", "queued", "done", "failed"];
  var FILTER_KEYS = { all: "filterAll", running: "filterRunning", queued: "filterQueued", done: "filterDone", failed: "filterFailed" };
  function dlCounts() {
    var live = Array.from(s.live.values());
    return {
      running: live.filter(function (l) { return l.state === "running"; }).length,
      queued: live.filter(function (l) { return l.state === "queued"; }).length,
      done: s.history.filter(function (h) { return h.state === "done" || h.state === "warning"; }).length,
      failed: s.history.filter(function (h) { return h.state === "fail" || h.state === "cancelled"; }).length,
      all: live.length + s.history.length
    };
  }

  function whenText(seconds) {
    var d = new Date(seconds * 1000), now = new Date();
    var hm = d.toLocaleTimeString(LANGUAGES.filter(function (l) { return l[0] === s.lang; })[0][2], { hour: "2-digit", minute: "2-digit" });
    var day = function (x) { return new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime(); };
    var diff = Math.round((day(now) - day(d)) / 86400000);
    if (diff === 0) return t("today", hm);
    if (diff === 1) return t("yesterday", hm);
    return d.toLocaleDateString(LANGUAGES.filter(function (l) { return l[0] === s.lang; })[0][2]) + " " + hm;
  }

  function folderName(path) { return String(path).split(/[\\/]/).filter(Boolean).pop() || path; }

  function dlButton(label, title, onClick) {
    var b = chip(label, onClick, title);
    return b;
  }

  function liveItem(l) {
    var li = el("li", "dl");
    li.appendChild(makeThumb(l.kind, l.thumb, "").box);
    var main = el("div", "dl__main");
    main.appendChild(el("div", "dl__title", l.title));
    main.appendChild(el("div", "dl__sub" + (l.state === "running" ? " is-ok" : ""), l.state === "queued" ? t("queued") : (l.msg || t("downloading"))));
    if (l.state === "running") {
      var bar = el("div", "dl__bar"), fill = el("i");
      if (l.total) fill.style.width = Math.min(100, l.done * 100 / l.total) + "%";
      bar.appendChild(fill);
      main.appendChild(bar);
    }
    li.appendChild(main);
    var actions = el("div", "dl__actions");
    actions.appendChild(dlButton("■", t("stopItem"), function () { invoke("cancel_item", { id: l.id }); }));
    li.appendChild(actions);
    return li;
  }

  function historyItem(h) {
    var li = el("li", "dl");
    li.appendChild(makeThumb(h.kind, h.thumb, "").box);
    var main = el("div", "dl__main");
    main.appendChild(el("div", "dl__title", h.title || h.url));
    var ok = h.state === "done" || h.state === "warning";
    var text = ok ? t("doneWhen", whenText(h.when), folderName(h.folder)) + (h.bytes ? " · " + formatSize(h.bytes) : "")
      : (h.state === "cancelled" ? t("cancelled") : tr(h.message)) + " · " + whenText(h.when);
    var sub = el("div", "dl__sub " + (ok ? "is-ok" : "is-fail"), text);
    sub.title = text;
    main.appendChild(sub);
    li.appendChild(main);
    var actions = el("div", "dl__actions");
    if (ok) {
      actions.appendChild(dlButton("▶", t("openFile"), function () { invoke("open_path", { path: h.files[0] || h.folder }).then(null, function (e) { setStatus(e, "error"); }); }));
      actions.appendChild(dlButton("📁", t("showInFolder"), function () { invoke("reveal_path", { path: h.files[0] || h.folder }).then(null, function (e) { setStatus(e, "error"); }); }));
    } else {
      actions.appendChild(dlButton("↻", t("tryAgain"), function () { retry(h); }));
    }
    var remove = makeRemoveButton(function () {
      s.history = s.history.filter(function (x) { return x.id !== h.id; });
      invoke("history_remove", { id: h.id });
      renderDownloads();
    });
    remove.className = "chipbtn";
    remove.title = t("removeEntry");
    actions.appendChild(remove);
    li.appendChild(actions);
    return li;
  }

  // Tentar de novo baixa na mesma pasta, com a mesma qualidade.
  function retry(h) {
    var item = { id: s.nextId++, url: h.url, kind: h.kind, folder: "", quality: h.quality || "1080", title: h.title, thumb: h.thumb, subs: false };
    trackLive(item, "queued");
    if (!s.running) { s.running = true; s.runningKind = "download"; ui.track.hidden = false; }
    refreshControls();
    renderDownloads();
    invoke("start_job", { request: { model: "", name: "", dest: h.folder, confirmExisting: false, convertTo: "", downloadOnly: true, items: [item] } }).then(function (answer) {
      if (answer.status === "queued") setStatus("queuedBatch");
    }, function (error) {
      s.live.delete(item.id);
      if (!s.live.size) { s.running = false; s.runningKind = ""; ui.track.hidden = true; }
      refreshControls();
      renderDownloads();
      setStatus(error, "error");
    });
  }

  var dlTimer = null;
  function renderDownloadsSoon() {
    if (dlTimer) return;
    dlTimer = setTimeout(function () { dlTimer = null; renderDownloads(); }, 250);
  }

  function renderDownloads() {
    var counts = dlCounts();
    ui.dlBadge.hidden = !s.live.size;
    ui.dlBadge.textContent = String(s.live.size);
    while (ui.dlFilters.firstChild) ui.dlFilters.removeChild(ui.dlFilters.firstChild);
    FILTERS.forEach(function (f) {
      var b = el("button", f === "failed" && counts.failed ? "is-bad" : "", t(FILTER_KEYS[f], counts[f]));
      b.type = "button";
      b.setAttribute("role", "tab");
      b.setAttribute("aria-selected", String(s.dlFilter === f));
      b.addEventListener("click", function () { s.dlFilter = f; renderDownloads(); });
      ui.dlFilters.appendChild(b);
    });
    if (s.tab !== "downloads") return;
    var query = ui.dlSearch.value.trim().toLowerCase();
    var match = function (x) { return !query || (x.title + " " + x.url).toLowerCase().indexOf(query) >= 0; };
    var items = [];
    var f = s.dlFilter;
    Array.from(s.live.values()).filter(match).forEach(function (l) {
      if (f === "all" || f === l.state) items.push(liveItem(l));
    });
    s.history.filter(match).forEach(function (h) {
      var ok = h.state === "done" || h.state === "warning";
      if (f === "all" || (f === "done" && ok) || (f === "failed" && !ok)) items.push(historyItem(h));
    });
    var scroll = ui.dlList.scrollTop;
    ui.dlList.replaceChildren.apply(ui.dlList, items);
    ui.dlList.scrollTop = scroll;
    ui.dlEmpty.hidden = items.length > 0;
    ui.clearDone.disabled = !counts.done;
    fit();
  }

  function onHistory(entry) {
    // A mesma regra do Rust: tentar de novo o mesmo link na mesma pasta substitui a falha anterior.
    s.history = s.history.filter(function (h) {
      var replaced = h.url === entry.url && h.folder === entry.folder && h.state !== "done" && h.state !== "warning";
      return h.id !== entry.id && !replaced;
    });
    s.history.unshift(entry);
    renderDownloadsSoon();
  }

  // ---- login pelo Frey (Configuracoes)

  function renderFreyLogin() {
    var sites = s.freySites || [];
    ui.sFreyState.textContent = sites.length ? t("freyConnected", sites.map(function (x) { return SITE_NAME[x]; }).join(", ")) : t("freyNone");
    var on = !!(s.yt && s.yt.risk);
    [ui.sFreyYoutube, ui.sFreyInstagram].forEach(function (b) { b.disabled = !on; });
    ui.sFreyForget.hidden = !sites.length;
  }

  function onLoginSaved(sites) {
    s.freySites = sites || [];
    renderFreyLogin();
    setStatus(sites && sites.length ? "loginSaved" : "loginNotSaved", sites && sites.length ? "" : "warning");
    // Le de novo os links que pararam por falta de login.
    s.rows.forEach(function (r) {
      if (r.probe && r.probe.error && (LOGIN_OFF.indexOf(r.probe.error) >= 0 || LOGIN_MISSING.indexOf(r.probe.error) >= 0)) probe(r);
    });
  }

  // ---- aba Converter

  function addFiles(paths) {
    return invoke("only_files", { paths: paths }).then(function (files) {
      files.forEach(function (path) {
        if (s.files.some(function (f) { return f.path === path; })) return;
        var file = { id: s.nextId++, path: path, state: "pending", msg: [path], node: null };
        file.node = makeRow(ui.cRows, "kindFile", path.split(/[\\/]/).pop(), [], function () {
          s.files = s.files.filter(function (f) { return f !== file; });
        });
        s.files.push(file);
        paintRow(file);
      });
      if (paths.length && !files.length) setStatus("foldersNotConverted", "warning");
      refreshControls();
      fit();
    });
  }

  function startConvert() {
    var pending = s.files.filter(function (f) { return f.state !== "ok"; });
    if (!pending.length) { setStatus("err.noFiles", "warning"); return; }
    var undo = beginRun(pending, paintRow);
    invoke("start_convert", { request: {
      files: pending.map(function (f) { return { id: f.id, path: f.path }; }),
      ext: ui.cFormat.value, mode: ui.cMode.value, crf: Number(ui.cQuality.value), useGpu: ui.cGpu.checked, outDir: s.outDir
    } }).then(null, function (error) { undo(); setStatus(error, "error"); });
  }

  function onConvert(ev) {
    var file = ev.id != null ? find(s.files, ev.id) : null;
    switch (ev.event) {
      case "start":
        if (file) { file.state = "running"; file.msg = ["converting"]; paintRow(file); }
        ui.bar.style.width = "0";
        break;
      case "progress":
        setBar(ev.percent);
        if (file) {
          file.msg = [ev.how, Math.floor(ev.percent) + "%"];
          paintRow(file);
          ui.status.textContent = file.path.split(/[\\/]/).pop() + " · " + file.msg.map(tr).join(" · ");
        }
        break;
      case "ok":
        if (file) { file.state = "ok"; file.msg = [c("convertedTo", ev.output)]; paintRow(file); }
        s.convertOpen = true;
        fit();
        break;
      case "fail":
        if (file) { file.state = "fail"; file.msg = [ev.message]; paintRow(file); }
        fit();
        break;
      case "cancelled":
        if (file) { file.state = "cancelled"; file.msg = ["cancelled"]; paintRow(file); }
        break;
      case "done":
        s.running = false;
        s.runningKind = "";
        ui.track.hidden = true;
        refreshControls();
        s.files.forEach(paintRow);
        if (ev.cancelled) setStatus(c("convertCancelled", ev.ok), "warning");
        else if (ev.failed) setStatus(c("convertSomeFailed", ev.ok, ev.failed), "error");
        else setStatus(c("convertDone", ev.ok));
        break;
    }
  }

  // ---- aba Configuracoes. Tudo salva sozinho ao mudar.

  function currentModel() { return s.models[s.modelIndex]; }

  function renderModelSelect() {
    while (ui.sModel.firstChild) ui.sModel.removeChild(ui.sModel.firstChild);
    s.models.forEach(function (m, i) { addOption(ui.sModel, String(i), m.name || "—"); });
    ui.sModel.value = String(s.modelIndex);
  }

  function loadEditor() {
    var m = currentModel();
    if (!m) return;
    ui.sName.value = m.name;
    ui.sRoot.value = m.root;
    renderFolderList();
    refreshSettingsTree();
    refreshControls();
    fit();
  }

  function refreshSettingsTree() {
    var m = currentModel();
    if (!m || !s.savedNames.length) return;
    var seq = ++s.settingsSeq;
    // A previa le o que esta salvo: durante um nome invalido, mostra o ultimo estado valido.
    var saved = s.savedNames[s.modelIndex] || m.name;
    invoke("preview", { model: saved, name: ui.name.value.trim() || t("defaultName") }).then(function (p) {
      if (seq === s.settingsSeq) { fillTree(ui.sTree, p.root, p.folders); fit(); }
    }, function () {});
  }

  // Lista de pastas do modelo. O destino segue a linha marcada, mesmo renomeada.
  function destinationRow(m) {
    return m.destination ? m.folders.indexOf(m.destination) : -1;
  }

  function markDestination(m, index) {
    m.destination = index >= 0 && m.folders[index] ? m.folders[index].trim() : "";
    s.destRow = index;
  }

  function renderFolderList(focusIndex) {
    var m = currentModel();
    if (!m) return;
    s.destRow = destinationRow(m);
    while (ui.sFolderList.firstChild) ui.sFolderList.removeChild(ui.sFolderList.firstChild);
    m.folders.forEach(function (folder, index) { ui.sFolderList.appendChild(makeFolderRow(m, index)); });
    paintDestination();
    if (focusIndex != null) {
      var inputs = ui.sFolderList.querySelectorAll("input[type=text]");
      if (inputs[focusIndex]) { inputs[focusIndex].focus(); inputs[focusIndex].select(); inputs[focusIndex].scrollIntoView({ block: "nearest" }); }
    }
    fit();
  }

  function makeFolderRow(m, index) {
    var li = document.createElement("li");
    li.className = "folder-row";
    var radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "folderDestination";
    radio.setAttribute("data-i18n-title", "destChoose");
    radio.setAttribute("data-i18n-aria", "destChoose");
    radio.title = t("destChoose");
    radio.setAttribute("aria-label", t("destChoose"));
    radio.addEventListener("change", function () {
      markDestination(m, index);
      paintDestination();
      persist();
    });
    var input = document.createElement("input");
    input.type = "text";
    input.dir = "ltr";
    input.maxLength = 120;
    input.spellcheck = false;
    input.autocomplete = "off";
    input.value = m.folders[index];
    input.setAttribute("data-i18n-placeholder", "folderPlaceholder");
    input.placeholder = t("folderPlaceholder");
    input.addEventListener("input", function () {
      m.folders[index] = input.value;
      if (s.destRow === index) m.destination = input.value.trim();
      persistSoon();
    });
    input.addEventListener("keydown", function (e) {
      if (e.key === "Enter") { e.preventDefault(); addFolderAfter(m, index); }
    });
    var tag = document.createElement("span");
    tag.className = "folder-row__tag";
    tag.setAttribute("data-i18n", "destTag");
    tag.textContent = t("destTag");
    var remove = makeRemoveButton(function () { removeFolder(m, index); });
    li.appendChild(radio);
    li.appendChild(input);
    li.appendChild(tag);
    li.appendChild(remove);
    return li;
  }

  function paintDestination() {
    Array.prototype.forEach.call(ui.sFolderList.children, function (li, i) {
      var on = i === s.destRow;
      li.classList.toggle("is-destination", on);
      li.querySelector("input[type=radio]").checked = on;
      li.querySelector(".folder-row__tag").hidden = !on;
    });
  }

  function addFolderAfter(m, index) {
    var at = index + 1;
    m.folders.splice(at, 0, "");
    if (s.destRow >= at) s.destRow++;
    // A primeira pasta de um modelo ja nasce como destino.
    if (s.destRow < 0 && !m.folders.some(function (f, i) { return i !== at && f.trim(); })) s.destRow = at;
    var mark = s.destRow;
    renderFolderListKeeping(m, mark, at);
  }

  function removeFolder(m, index) {
    m.folders.splice(index, 1);
    var mark = s.destRow;
    if (mark === index) {
      // Sem a pasta marcada, o destino passa para a primeira que sobrou.
      mark = m.folders.length ? 0 : -1;
      m.destination = mark >= 0 ? m.folders[0].trim() : "";
    } else if (mark > index) {
      mark--;
    }
    renderFolderListKeeping(m, mark);
    persist();
  }

  // Reconstroi sem perder a linha marcada, mesmo com pasta ainda sem nome.
  function renderFolderListKeeping(m, mark, focusIndex) {
    renderFolderList();
    s.destRow = mark;
    paintDestination();
    if (focusIndex != null) {
      var inputs = ui.sFolderList.querySelectorAll("input[type=text]");
      if (inputs[focusIndex]) { inputs[focusIndex].focus(); inputs[focusIndex].scrollIntoView({ block: "nearest" }); }
    }
    fit();
  }

  // Mantem a aba Projeto em dia: o modelo escolhido continua, mesmo renomeado.
  function syncProjectModels(selectedIndex) {
    var selected = ui.model.value;
    while (ui.model.firstChild) ui.model.removeChild(ui.model.firstChild);
    s.models.forEach(function (m) { addOption(ui.model, m.name, m.name); });
    var names = s.models.map(function (m) { return m.name; });
    if (names.indexOf(selected) >= 0) ui.model.value = selected;
    else ui.model.value = names[Math.min(Math.max(selectedIndex, 0), names.length - 1)];
    refreshPreview();
  }

  function persist() {
    clearTimeout(s.saveTimer);
    s.saveTimer = null;
    var selectedIndex = s.savedNames.indexOf(ui.model.value);
    return invoke("save_models", { models: s.models, openAtEnd: ui.sOpenAtEnd.checked }).then(function () {
      s.savedNames = s.models.map(function (m) { return m.name.trim(); });
      s.models.forEach(function (m, i) { m.name = s.savedNames[i]; });
      syncProjectModels(selectedIndex);
      refreshSettingsTree();
      setStatus("saved");
    }, function (error) { setStatus(error, "error"); });
  }

  function persistSoon() {
    clearTimeout(s.saveTimer);
    s.saveTimer = setTimeout(persist, 350);
  }

  function selectModel(index) {
    s.modelIndex = index;
    renderModelSelect();
    loadEditor();
  }

  function uniqueName(base) {
    var used = s.models.map(function (m) { return m.name.toLowerCase(); });
    var name = base, n = 2;
    while (used.indexOf(name.toLowerCase()) >= 0) { name = base + " " + n; n++; }
    return name;
  }

  // Excluir e restaurar pedem um segundo clique, como no Frey.
  function disarm() {
    if (s.armed) clearTimeout(s.armed.timer);
    s.armed = null;
    ui.sDelete.textContent = t("modelDelete");
    ui.sRestore.textContent = t("restoreDefault");
  }

  function confirmClick(button, key, action) {
    if (s.armed && s.armed.button === button) { disarm(); action(); return; }
    disarm();
    button.textContent = t(key);
    s.armed = { button: button, timer: setTimeout(disarm, 3500) };
    fit();
  }

  function loadModels(answer) {
    s.models = answer.models;
    s.savedNames = s.models.map(function (m) { return m.name; });
    s.modelIndex = Math.min(s.modelIndex, s.models.length - 1);
    ui.sOpenAtEnd.checked = answer.openAtEnd;
    renderModelSelect();
    loadEditor();
  }

  // ---- eventos

  ui.createFolders.addEventListener("change", function () { setDownloadOnly(!ui.createFolders.checked); });
  ui.quick.addEventListener("change", function () {
    s.quick = ui.quick.checked;
    localStorage.setItem("fp.quick", s.quick ? "1" : "");
    if (s.quick && pendingRows().length) quickStart();
  });
  ui.tabProject.addEventListener("click", function () { selectTab("project"); focusProject(); });
  ui.tabDownloads.addEventListener("click", function () { selectTab("downloads"); });
  ui.dlSearch.addEventListener("input", renderDownloads);
  ui.clearDone.addEventListener("click", function () {
    invoke("history_clear_done").then(function (list) { s.history = list; renderDownloads(); });
  });
  ui.sFreyYoutube.addEventListener("click", function () { invoke("open_login", { site: "youtube" }).then(null, function (e) { setStatus(e, "error"); }); });
  ui.sFreyInstagram.addEventListener("click", function () { invoke("open_login", { site: "instagram" }).then(null, function (e) { setStatus(e, "error"); }); });
  ui.sFreyForget.addEventListener("click", function () {
    confirmClick(ui.sFreyForget, "freyForgetConfirm", function () {
      invoke("forget_frey_login").then(function (sites) { s.freySites = sites; renderFreyLogin(); ui.sFreyForget.textContent = t("freyForget"); });
    });
  });
  ui.tabConvert.addEventListener("click", function () { selectTab("convert"); });
  ui.tabSettings.addEventListener("click", function () { selectTab("settings"); });
  ui.go.addEventListener("click", function () {
    if (s.tab === "convert") { startConvert(); return; }
    var confirm = s.confirmNext;
    s.confirmNext = false;
    startProject(confirm);
  });
  ui.cancel.addEventListener("click", function () { invoke("cancel_job"); setStatus("cancelling"); });
  ui.open.addEventListener("click", function () { invoke(s.tab === "convert" ? "open_converted" : "open_project"); });
  ui.hide.addEventListener("click", function () { invoke("hide_panel"); });
  ui.dest.addEventListener("click", function () {
    invoke("pick_folder", { current: s.dest }).then(function (path) {
      if (path) { setDest(path); clearConfirm(); }
    });
  });
  ui.destMore.addEventListener("click", function () { toggleRecent(!s.recentOpen); });
  // Clique fora da lista fecha a lista.
  document.addEventListener("mousedown", function (e) {
    if (s.recentOpen && !ui.recent.contains(e.target) && !ui.destMore.contains(e.target)) toggleRecent(false);
  });
  ui.cOut.addEventListener("click", function () {
    invoke("pick_out_dir").then(function (path) {
      s.outDir = path || "";
      ui.cOut.textContent = s.outDir || t("sameFolder");
      ui.cOut.classList.toggle("is-empty", !s.outDir);
    });
  });
  ui.cAdd.addEventListener("click", function () { invoke("pick_files").then(addFiles); });
  ui.cFormat.addEventListener("change", syncModeControl);

  function clearConfirm() {
    if (s.confirmNext) { s.confirmNext = false; setStatus(""); refreshControls(); }
  }
  ui.model.addEventListener("change", function () { clearConfirm(); refreshPreview(); });
  ui.name.addEventListener("input", function () {
    clearConfirm();
    if (s.status === "err.nameNeeded") setStatus("");
    refreshPreview();
  });

  ui.sLanguage.addEventListener("change", function () {
    s.lang = ui.sLanguage.value;
    applyLanguage();
    refreshPreview();
    invoke("set_language", { language: s.lang });
  });
  ui.sOpenAtEnd.addEventListener("change", persist);
  ui.sDetectLink.addEventListener("change", function () { invoke("set_detect_link", { detect: ui.sDetectLink.checked }); });
  // O aviso de risco fica atras do icone de alerta e so abre quando o usuario pede.
  var riskInfo = document.getElementById("riskInfo"), riskText = document.getElementById("riskText");
  riskInfo.addEventListener("click", function () {
    var open = riskText.hidden;
    riskText.hidden = !open;
    riskInfo.setAttribute("aria-expanded", String(open));
    fit();
  });
  ui.sYtRisk.addEventListener("change", function () {
    s.yt.risk = ui.sYtRisk.checked;
    if (!s.yt.risk) s.yt.source = "";
    // Ao aceitar, o login ja comeca no automatico: procura a conta em todos os navegadores.
    else if (!s.yt.source) s.yt.source = "auto";
    saveYoutubeLogin();
  });
  ui.sYtLogin.addEventListener("change", function () { s.yt.source = ui.sYtLogin.value; saveYoutubeLogin(); });
  ui.sYtCookies.addEventListener("click", function () {
    invoke("pick_cookies_file").then(function (path) { if (path) { s.yt.file = path; saveYoutubeLogin(); } });
  });
  ui.sModel.addEventListener("change", function () {
    if (s.saveTimer) persist();
    disarm();
    selectModel(Number(ui.sModel.value));
  });
  ui.sName.addEventListener("input", function () {
    currentModel().name = ui.sName.value;
    if (ui.sModel.options[s.modelIndex]) ui.sModel.options[s.modelIndex].textContent = ui.sName.value || "—";
    persistSoon();
  });
  ui.sRoot.addEventListener("input", function () { currentModel().root = ui.sRoot.value; persistSoon(); });
  ui.sAddFolder.addEventListener("click", function () {
    var m = currentModel();
    addFolderAfter(m, m.folders.length - 1);
  });
  ui.sNew.addEventListener("click", function () {
    s.models.push({ name: uniqueName(t("newModel")), root: "{nome}", folders: [], destination: "", files: [] });
    selectModel(s.models.length - 1);
    persist();
    ui.sName.focus();
    ui.sName.select();
  });
  ui.sDuplicate.addEventListener("click", function () {
    var m = currentModel();
    s.models.push({ name: uniqueName(t("copyOf", m.name)), root: m.root, folders: m.folders.slice(), destination: m.destination, files: m.files.slice() });
    selectModel(s.models.length - 1);
    persist();
    ui.sName.focus();
    ui.sName.select();
  });
  ui.sDelete.addEventListener("click", function () {
    confirmClick(ui.sDelete, "modelDeleteConfirm", function () {
      if (s.models.length <= 1) return;
      s.models.splice(s.modelIndex, 1);
      s.savedNames.splice(s.modelIndex, 1);
      selectModel(Math.min(s.modelIndex, s.models.length - 1));
      persist();
    });
  });
  ui.sRestore.addEventListener("click", function () {
    confirmClick(ui.sRestore, "restoreConfirm", function () {
      invoke("restore_default_model").then(function (answer) {
        var wasBroken = s.broken;
        s.broken = false;
        var selectedIndex = s.savedNames.indexOf(ui.model.value);
        loadModels(answer);
        syncProjectModels(selectedIndex);
        if (wasBroken && !ui.name.value) ui.name.value = t("defaultName");
        refreshControls();
        setStatus("saved");
      }, function (error) { setStatus(error, "error"); });
    });
  });

  // Colar link em qualquer ponto da aba Projeto adiciona a lista; texto comum segue para o campo.
  document.addEventListener("paste", function (e) {
    if (s.tab !== "project") return;
    var text = e.clipboardData ? e.clipboardData.getData("text") : "";
    if (!/(drive|docs)\.google\.com\/|youtube\.com\/|youtu\.be\/|(x|twitter)\.com\/|instagram\.com\/|tiktok\.com\//.test(text) && e.target !== ui.links) return;
    e.preventDefault();
    addLinks(text).then(function (count) {
      setStatus(count ? "" : "noLinkFound", count ? "" : "warning");
    });
  });

  ui.links.addEventListener("keydown", function (e) {
    if (e.key !== "Enter" || !ui.links.value.trim()) return;
    e.stopPropagation();
    addLinks(ui.links.value).then(function (count) {
      if (count) ui.links.value = "";
      else setStatus("noLinkFound", "warning");
    });
  });

  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && s.recentOpen) { toggleRecent(false); ui.destMore.focus(); return; }
    if (e.key === "Escape") { invoke("hide_panel"); return; }
    if (e.key === "Enter" && !ui.go.disabled && (e.target === ui.name || e.target === ui.links)) ui.go.click();
  });

  // Soltar: na aba Projeto, pasta vira destino; na Converter, arquivos entram na lista.
  listen("tauri://drag-enter", function () { ui.shell.classList.add("is-dropping"); });
  listen("tauri://drag-leave", function () { ui.shell.classList.remove("is-dropping"); });
  listen("tauri://drag-drop", function (e) {
    ui.shell.classList.remove("is-dropping");
    var paths = e.payload.paths || [];
    if (!paths.length || s.tab === "settings" || s.tab === "downloads") return;
    if (s.tab === "convert") { if (!s.running) addFiles(paths); return; }
    invoke("is_folder", { path: paths[0] }).then(function (folder) {
      if (folder) { setDest(paths[0]); clearConfirm(); }
      else setStatus("dropFolderHint", "warning");
    });
  });

  listen("job", function (e) { onJob(e.payload); });
  listen("history", function (e) { onHistory(e.payload); });
  listen("login-saved", function (e) { onLoginSaved(e.payload); });
  listen("convert", function (e) { onConvert(e.payload); });
  // Link recem-copiado fica pronto na lista; os ja concluidos saem. Nunca baixa sozinho.
  function takeCopiedLinks() {
    if (s.broken) return;
    invoke("take_copied_links").then(function (links) {
      if (!links.length) return;
      var known = links.every(function (link) { return s.rows.some(function (r) { return r.url === link.url && r.state !== "ok" && r.state !== "warning"; }); });
      if (known) return;
      s.rows.filter(function (r) { return r.state === "ok" || r.state === "warning"; }).forEach(function (r) { r.node.li.remove(); });
      s.rows = s.rows.filter(function (r) { return r.state !== "ok" && r.state !== "warning"; });
      addLinks(links.map(function (link) { return link.url; }).join("\n")).then(function () {
        if (s.tab !== "project") selectTab("project");
        setStatus("linkDetected");
        focusProject();
      });
    });
  }

  listen("panel-shown", function () {
    refreshSubfolders();
    focusProject();
    takeCopiedLinks();
    fit();
  });

  // ---- inicio

  LANGUAGES.forEach(function (l) { addOption(ui.sLanguage, l[0], l[1]); });

  // "cmd+alt+p" vira "⌘⌥P" no Mac e "Ctrl+Alt+P" no Windows.
  function hotkeyText(hotkey) {
    var mac = { CMD: "⌘", COMMAND: "⌘", SUPER: "⌘", ALT: "⌥", OPTION: "⌥", CTRL: "⌃", CONTROL: "⌃", SHIFT: "⇧" };
    var win = { CTRL: "Ctrl", CONTROL: "Ctrl", ALT: "Alt", SHIFT: "Shift", SUPER: "Win" };
    var parts = hotkey.toUpperCase().split("+").map(function (p) { return (s.mac ? mac : win)[p] || p; });
    return parts.join(s.mac ? "" : "+");
  }

  invoke("boot").then(function (b) {
    s.mac = b.platform === "macos";
    ui.hotkey.textContent = hotkeyText(b.hotkey);
    s.lang = pickLanguage(b.language);
    s.downloadOnly = b.downloadOnly;
    ui.sDetectLink.checked = b.detectLink;
    s.yt = { source: b.youtubeLogin, file: b.youtubeCookies, risk: b.youtubeRisk };
    s.quick = localStorage.getItem("fp.quick") === "1";
    ui.quick.checked = s.quick;
    invoke("history_list").then(function (list) { s.history = list; renderDownloads(); });
    invoke("login_state").then(function (sites) { s.freySites = sites; renderFreyLogin(); });
    ui.sLanguage.value = s.lang;
    if (s.lang !== b.language) invoke("set_language", { language: s.lang });
    if (b.configError) {
      // Modelos com erro: o Converter segue funcionando e Configuracoes oferece restaurar.
      s.broken = true;
      setDest(b.dest);
      applyLanguage();
      applyMode();
      selectTab("settings");
      setStatus(b.configError, "error");
      return;
    }
    b.models.forEach(function (name) { addOption(ui.model, name, name); });
    ui.model.value = b.model;
    ui.name.value = b.defaultName;
    setDest(b.dest);
    s.running = b.running;
    if (b.running) s.runningKind = "download";
    applyLanguage();
    applyMode();
    if (b.hotkeyError) setStatus(b.hotkeyError, "warning");
    invoke("get_models").then(function (answer) {
      loadModels(answer);
      selectModel(Math.max(0, s.savedNames.indexOf(b.model)));
    });
    refreshPreview();
    // Focar com o painel escondido levaria o teclado do Windows para uma janela invisivel.
    if (b.visible) focusProject();
  }, function (error) { setStatus(error, "error"); });
}());
