"use strict";
// Painel do Frey Project. Toda a logica de disco, download e conversao fica no Rust; aqui so ha exibicao.
(function () {
  var invoke = window.__TAURI__.core.invoke;
  var listen = window.__TAURI__.event.listen;
  var STRINGS = window.FreyStrings;
  var $ = function (id) { return document.getElementById(id); };
  var LANGUAGES = [["pt", "Português", "pt-BR"], ["en", "English", "en"], ["es", "Español", "es"], ["fr", "Français", "fr"], ["ja", "日本語", "ja"], ["zh", "简体中文", "zh-Hans"], ["zht", "繁體中文", "zh-Hant"], ["ru", "Русский", "ru"], ["ar", "العربية", "ar"]];
  var FORMATS = [["mp4", "MP4", "fmtVideo"], ["mov", "MOV", "fmtVideo"], ["mkv", "MKV", "fmtVideo"], ["webm", "WEBM", "fmtVideo"], ["gif", "GIF", "fmtAnimated"], ["mp3", "MP3", "fmtAudio"], ["wav", "WAV", "fmtAudio"], ["m4a", "M4A", "fmtAudio"]];
  var QUALITIES = [["1080", "q1080"], ["720", "q720"], ["edit4k", "qEdit4k"], ["best", "qBest"], ["audio", "qAudio"]];
  var SEP = "\u001f";

  var ui = {
    shell: $("shell"), name: $("name"), model: $("model"), dest: $("dest"), tree: $("tree"), links: $("links"),
    rows: $("rows"), status: $("status"), go: $("go"), cancel: $("cancel"), open: $("open"), hide: $("hide"), pin: $("pin"), actions: $("actions"),
    track: $("progressTrack"), bar: $("progressBar"), hotkey: $("hotkey"), convertTo: $("convertTo"),
    modeCreate: $("modeCreate"), modeDownload: $("modeDownload"), queryRow: $("queryRow"), fieldModel: $("fieldModel"), destLabel: $("destLabel"),
    tabProject: $("tabProject"), tabConvert: $("tabConvert"), tabSettings: $("tabSettings"),
    viewProject: $("viewProject"), viewConvert: $("viewConvert"), viewSettings: $("viewSettings"),
    cFormat: $("cFormat"), cMode: $("cMode"), cQuality: $("cQuality"), cOut: $("cOut"), cGpu: $("cGpu"), cGpuLabel: $("cGpuLabel"), cAdd: $("cAdd"), cRows: $("cRows"),
    sLanguage: $("sLanguage"), sOpenAtEnd: $("sOpenAtEnd"), sModel: $("sModel"), sNew: $("sNew"), sDuplicate: $("sDuplicate"), sDelete: $("sDelete"),
    sName: $("sName"), sRoot: $("sRoot"), sFolderList: $("sFolderList"), sAddFolder: $("sAddFolder"), sTree: $("sTree"), sRestore: $("sRestore")
  };
  var s = {
    lang: "pt", tab: "project", pinned: false, running: false, projectOpen: false, convertOpen: false, confirmNext: false, broken: false,
    rows: [], files: [], nextId: 1, folders: [], defaultFolder: "", dest: "", outDir: "", requestSeq: 0, settingsSeq: 0, toolsChecked: false, gpu: "detectingGpu",
    status: "", statusKind: "", root: "", downloadOnly: false, needsName: true, models: [], savedNames: [], modelIndex: 0, saveTimer: null, armed: null
  };

  // ---- traducao

  function t(key) {
    var text = STRINGS[s.lang][key];
    if (text == null) return "[[" + key + "]]";
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
    invoke("resize_panel", { height: Math.ceil(ui.shell.getBoundingClientRect().height) });
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
  }

  function syncPin() {
    // Na aba Converter o painel precisa ficar aberto para receber arquivos arrastados do Explorer.
    invoke("set_pinned", { pinned: s.pinned || s.tab === "convert" });
    ui.pin.setAttribute("aria-pressed", String(s.pinned));
  }

  function refreshControls() {
    if (s.tab === "convert") {
      var again = s.files.some(function (f) { return f.state === "fail" || f.state === "cancelled"; });
      ui.go.textContent = t(s.running ? "working" : again ? "tryAgain" : "convertBtn");
      ui.open.disabled = !s.convertOpen;
    } else {
      var failed = s.rows.some(function (r) { return r.state === "fail" || r.state === "cancelled"; });
      if (s.running) ui.go.textContent = t("working");
      else if (s.confirmNext) ui.go.textContent = t("completeExisting");
      else if (failed) ui.go.textContent = t("tryAgain");
      else if (s.downloadOnly) ui.go.textContent = t("downloadBtn");
      else ui.go.textContent = t(s.rows.length ? "createAndDownload" : "createProject");
      ui.open.disabled = !s.projectOpen;
    }
    ui.go.disabled = s.running || (s.broken && s.tab === "project" && !s.downloadOnly);
    ui.cancel.hidden = !s.running;
    ui.actions.hidden = s.tab === "settings" && !s.running;
    // Modelos com erro so travam o que depende deles: baixar continua possivel.
    [ui.name, ui.model].forEach(function (n) { n.disabled = s.running || s.broken; });
    [ui.dest, ui.links, ui.convertTo, ui.modeCreate, ui.modeDownload].forEach(function (n) { n.disabled = s.running; });
    [ui.cFormat, ui.cMode, ui.cQuality, ui.cOut, ui.cAdd].forEach(function (n) { n.disabled = s.running; });
    if (!s.running) syncModeControl();
    [ui.sModel, ui.sNew, ui.sDuplicate, ui.sName, ui.sRoot, ui.sAddFolder, ui.sOpenAtEnd].forEach(function (n) { n.disabled = s.broken; });
    ui.sDelete.disabled = s.broken || s.models.length <= 1;
  }

  // Criar pastas pede o nome so se o modelo tiver onde usar; So baixar esconde o que e da estrutura.
  function applyMode() {
    ui.modeCreate.setAttribute("aria-checked", String(!s.downloadOnly));
    ui.modeDownload.setAttribute("aria-checked", String(s.downloadOnly));
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
    if (s.running || s.tab !== "project") return;
    if (!s.downloadOnly && s.needsName && !s.broken) { ui.name.focus(); ui.name.select(); }
    else ui.links.focus();
  }

  function setDownloadOnly(value) {
    s.downloadOnly = value;
    s.confirmNext = false;
    if (s.status === "err.nameNeeded" || s.status === "err.noLinks") setStatus("");
    applyMode();
    focusProject();
    invoke("set_download_only", { downloadOnly: value });
  }

  function syncModeControl() {
    ui.cMode.disabled = ["mp4", "mov", "mkv"].indexOf(ui.cFormat.value) < 0;
  }

  function selectTab(tab) {
    s.tab = tab;
    [["project", ui.tabProject, ui.viewProject], ["convert", ui.tabConvert, ui.viewConvert], ["settings", ui.tabSettings, ui.viewSettings]].forEach(function (x) {
      x[1].setAttribute("aria-selected", String(x[0] === tab));
      x[2].hidden = x[0] !== tab;
    });
    syncPin();
    disarm();
    refreshControls();
    if (tab === "convert") checkTools();
    if (tab === "settings") refreshSettingsTree();
    if (tab !== "settings" && s.status === "saved") setStatus("");
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

  // ---- aba Projeto

  function folderOptions(select, value) {
    while (select.firstChild) select.removeChild(select.firstChild);
    addOption(select, "", t("projectRoot"));
    s.folders.forEach(function (folder) { addOption(select, folder, folder); });
    select.value = value;
  }

  function qualityOptions(select, value) {
    while (select.firstChild) select.removeChild(select.firstChild);
    QUALITIES.forEach(function (q) { addOption(select, q[0], t(q[1])); });
    select.title = t("qualityHint");
    select.setAttribute("aria-label", t("youtubeQuality"));
    select.value = value;
  }

  function renderLink(row) {
    if (!row.node) {
      var select = document.createElement("select");
      select.addEventListener("change", function () { row.folder = select.value; row.folderChosen = true; });
      var selects = [select];
      if (row.kind === "youtube") {
        var quality = document.createElement("select");
        quality.className = "row__quality";
        quality.addEventListener("change", function () { row.quality = quality.value; });
        row.qualitySelect = quality;
        selects.unshift(quality);
      }
      var kinds = { folder: "kindFolder", youtube: "kindYoutube", file: "kindFile" };
      row.node = makeRow(ui.rows, kinds[row.kind], row.url, selects, function () {
        s.rows = s.rows.filter(function (r) { return r !== row; });
      });
    }
    paintRow(row);
  }

  function renderRows() {
    s.rows.forEach(function (row) {
      // Link que o usuario nao mudou acompanha a pasta de destino do modelo.
      if (!row.folderChosen && row.state === "pending") row.folder = s.defaultFolder;
      if (s.folders.indexOf(row.folder) < 0 && row.folder !== "") row.folder = s.defaultFolder;
      renderLink(row);
      folderOptions(row.node.folder, row.folder);
      row.node.folder.setAttribute("aria-label", t("downloadTo"));
      if (row.qualitySelect) qualityOptions(row.qualitySelect, row.quality);
    });
    fit();
  }

  function fillTree(target, root, folders) {
    while (target.firstChild) target.removeChild(target.firstChild);
    var b = document.createElement("b");
    b.textContent = (root || t("chosenFolder")) + "\\  ";
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

  function addLinks(text) {
    return invoke("parse_links", { text: text }).then(function (links) {
      links.forEach(function (link) {
        if (s.rows.some(function (r) { return r.url === link.url; })) return;
        s.rows.push({ id: s.nextId++, url: link.url, kind: link.kind, folder: s.defaultFolder, quality: "1080", state: "pending", msg: ["waiting"], node: null });
      });
      renderRows();
      refreshControls();
      return links.length;
    });
  }

  // Entra em execucao e devolve a funcao que desfaz, se o Rust nao iniciar o trabalho.
  // Marca antes de chamar: os eventos do Rust podem chegar antes da resposta do invoke.
  function beginRun(items, paint) {
    var saved = items.map(function (r) { return { row: r, state: r.state, msg: r.msg }; });
    s.running = true;
    items.forEach(function (r) { r.state = "queued"; r.msg = ["queued"]; paint(r); });
    ui.track.hidden = false;
    ui.bar.style.width = "0";
    setStatus("preparing");
    refreshControls();
    return function () {
      s.running = false;
      ui.track.hidden = true;
      saved.forEach(function (o) { o.row.state = o.state; o.row.msg = o.msg; paint(o.row); });
      setStatus("");
      refreshControls();
    };
  }

  function startProject(confirmExisting) {
    var pending = s.rows.filter(function (r) { return r.state !== "ok" && r.state !== "warning"; });
    var undo = beginRun(pending, renderLink);
    invoke("start_job", { request: {
      model: ui.model.value, name: ui.name.value, dest: s.dest, confirmExisting: confirmExisting, convertTo: ui.convertTo.value, downloadOnly: s.downloadOnly,
      items: pending.map(function (r) { return { id: r.id, url: r.url, kind: r.kind, folder: r.folder, quality: r.quality }; })
    } }).then(function (answer) {
      if (answer.status === "needs_confirm") {
        undo();
        s.confirmNext = true;
        setStatus(c("existingConfirm", answer.root), "warning");
        refreshControls();
        return;
      }
      s.projectOpen = true;
      refreshControls();
      if (answer.status === "created") { undo(); setStatus(c("createdAt", answer.base)); }
    }, function (error) {
      undo();
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

  function onJob(ev) {
    var row = ev.id != null ? find(s.rows, ev.id) : null;
    switch (ev.event) {
      case "start":
        if (row) { row.state = "running"; row.msg = ["downloading"]; renderLink(row); }
        ui.bar.style.width = "0";
        break;
      case "progress": {
        var p = ev.progress;
        var parts = [p.file];
        if (p.filesTotal > 1) parts.unshift(t("fileOf", p.filesDone + 1, p.filesTotal));
        if (p.total) {
          var percent = Math.floor(p.done * 100 / p.total);
          parts.push(t("sizeOf", percent, formatSize(p.done), formatSize(p.total)));
          setBar(percent);
        } else {
          parts.push(formatSize(p.done));
          ui.bar.classList.add("is-indeterminate");
        }
        ui.status.textContent = parts.join(" · ");
        break;
      }
      case "converting":
        if (row) { row.msg = ["converting"]; renderLink(row); }
        setBar(ev.percent);
        ui.status.textContent = t("convertingFile", ev.file, Math.floor(ev.percent));
        break;
      case "ok":
        if (row) {
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
        if (row) { row.state = "fail"; row.msg = [ev.message]; renderLink(row); }
        fit();
        break;
      case "cancelled":
        if (row) { row.state = "cancelled"; row.msg = ["cancelledKept"]; renderLink(row); }
        break;
      case "done":
        s.running = false;
        ui.track.hidden = true;
        refreshControls();
        s.rows.forEach(renderLink);
        if (ev.cancelled) setStatus(c("downloadCancelled", ev.ok), "warning");
        else if (ev.failed) setStatus(c("downloadSomeFailed", ev.ok, ev.failed), "error");
        else setStatus(c("downloadDone", ev.ok, ev.base));
        break;
    }
  }

  // ---- aba Converter

  function addFiles(paths) {
    return invoke("only_files", { paths: paths }).then(function (files) {
      files.forEach(function (path) {
        if (s.files.some(function (f) { return f.path === path; })) return;
        var file = { id: s.nextId++, path: path, state: "pending", msg: [path], node: null };
        file.node = makeRow(ui.cRows, "kindFile", path.split("\\").pop(), [], function () {
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
          ui.status.textContent = file.path.split("\\").pop() + " · " + file.msg.map(tr).join(" · ");
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

  ui.modeCreate.addEventListener("click", function () { setDownloadOnly(false); });
  ui.modeDownload.addEventListener("click", function () { setDownloadOnly(true); });
  ui.tabProject.addEventListener("click", function () { selectTab("project"); focusProject(); });
  ui.tabConvert.addEventListener("click", function () { selectTab("convert"); });
  ui.tabSettings.addEventListener("click", function () { selectTab("settings"); });
  ui.pin.addEventListener("click", function () { s.pinned = !s.pinned; syncPin(); });
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
    s.models.push({ name: uniqueName(t("newModel")), root: "{data}_{nome}", folders: [], destination: "", files: [] });
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
    if (s.running || s.tab !== "project") return;
    var text = e.clipboardData ? e.clipboardData.getData("text") : "";
    if (!/(drive|docs)\.google\.com\/|youtube\.com\/|youtu\.be\//.test(text) && e.target !== ui.links) return;
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
    if (e.key === "Escape") { invoke("hide_panel"); return; }
    if (e.key === "Enter" && !ui.go.disabled && (e.target === ui.name || e.target === ui.links)) ui.go.click();
  });

  // Soltar: na aba Projeto, pasta vira destino; na Converter, arquivos entram na lista.
  listen("tauri://drag-enter", function () { ui.shell.classList.add("is-dropping"); });
  listen("tauri://drag-leave", function () { ui.shell.classList.remove("is-dropping"); });
  listen("tauri://drag-drop", function (e) {
    ui.shell.classList.remove("is-dropping");
    var paths = e.payload.paths || [];
    if (s.running || !paths.length || s.tab === "settings") return;
    if (s.tab === "convert") { addFiles(paths); return; }
    invoke("is_folder", { path: paths[0] }).then(function (folder) {
      if (folder) { setDest(paths[0]); clearConfirm(); }
      else setStatus("dropFolderHint", "warning");
    });
  });

  listen("job", function (e) { onJob(e.payload); });
  listen("convert", function (e) { onConvert(e.payload); });
  listen("panel-shown", function () {
    focusProject();
    fit();
  });

  // ---- inicio

  LANGUAGES.forEach(function (l) { addOption(ui.sLanguage, l[0], l[1]); });

  invoke("boot").then(function (b) {
    ui.hotkey.textContent = b.hotkey.toUpperCase().replace(/CTRL/, "Ctrl").replace(/ALT/, "Alt").replace(/SHIFT/, "Shift");
    s.lang = pickLanguage(b.language);
    s.downloadOnly = b.downloadOnly;
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
