// Confere os 9 idiomas contra o codigo real: nenhuma chave faltando, sobrando ou com marcador trocado.
// Uso: node --test tests/strings.test.cjs
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const root = path.join(__dirname, "..");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(read("ui/strings.js"), sandbox);
const strings = sandbox.window.FreyStrings;
const languages = ["pt", "en", "es", "fr", "ja", "zh", "zht", "ru", "ar"];
const keys = Object.keys(strings.pt);
const marks = (text) => (text.match(/\{\d\}/g) || []).sort().join();
const tokens = (text) => (text.match(/\{[a-z]+\}/g) || []).sort().join();

test("os 9 idiomas do Frey, na mesma ordem", () => {
  assert.deepStrictEqual(Object.keys(strings), languages);
});

test("cada idioma tem exatamente as mesmas chaves, sem texto vazio", () => {
  for (const lang of languages) {
    assert.deepStrictEqual(Object.keys(strings[lang]).sort(), [...keys].sort(), lang);
    for (const key of keys) assert.ok(strings[lang][key].trim(), `${lang}.${key}`);
  }
});

test("marcadores {0} e tokens {nome} iguais em todo idioma", () => {
  for (const lang of languages) {
    for (const key of keys) {
      assert.strictEqual(marks(strings[lang][key]), marks(strings.pt[key]), `${lang}.${key}: ${strings[lang][key]}`);
      assert.strictEqual(tokens(strings[lang][key]), tokens(strings.pt[key]), `${lang}.${key}: ${strings[lang][key]}`);
    }
  }
});

test("nenhuma traducao ficou como copia do portugues ou do ingles", () => {
  const universal = new Set(["kindYoutube", "q1080", "q720", "qNormal", "err.ffmpegFailed", "err.ytFailed", "language", "modeLabel", "modelLabel", "cancel", "cancelling", "tabConvert", "convertBtn", "convertTo", "preparing", "conv.copy"]);
  for (const lang of ["es", "fr", "ja", "zh", "zht", "ru", "ar"]) {
    const same = keys.filter((k) => !universal.has(k) && strings[lang][k] === strings.en[k]);
    assert.deepStrictEqual(same, [], `${lang} igual ao ingles`);
  }
});

test("toda chave usada no HTML existe", () => {
  const html = read("ui/index.html");
  for (const m of html.matchAll(/data-i18n(?:-placeholder|-title|-aria)?="([^"]+)"/g)) assert.ok(keys.includes(m[1]), m[1]);
});

test("toda chave usada no panel.js existe, e toda chave existente e usada", () => {
  const js = read("ui/panel.js");
  const html = read("ui/index.html");
  const rust = ["main", "drive", "convert", "youtube", "models"].map((f) => read(`src-tauri/src/${f}.rs`)).join("\n");
  const used = new Set();
  for (const m of js.matchAll(/\b(?:t|c|setStatus|setStatusKey)\("([A-Za-z.]+)"/g)) used.add(m[1]);
  for (const m of js.matchAll(/"([a-zA-Z0-9]+)"/g)) if (keys.includes(m[1])) used.add(m[1]);
  for (const m of js.matchAll(/"(err\.[A-Za-z]+)"/g)) used.add(m[1]);
  for (const key of used) if (/^(err|conv)\./.test(key) || /^[a-z]+[A-Z]/.test(key)) { if (js.includes(`"${key}"`) && (key.startsWith("err.") || key.startsWith("conv."))) assert.ok(keys.includes(key), key); }
  for (const m of js.matchAll(/\bt\("([A-Za-z.]+)"/g)) assert.ok(keys.includes(m[1]), m[1]);
  for (const key of keys) {
    const found = js.includes(`"${key}"`) || html.includes(`"${key}"`) || rust.includes(`"${key}"`);
    assert.ok(found, `chave sem uso: ${key}`);
  }
});

test("toda mensagem vinda do Rust tem traducao", () => {
  const rust = ["main", "drive", "convert", "youtube", "models"].map((f) => read(`src-tauri/src/${f}.rs`)).join("\n").split("#[cfg(test)]").map((part, i) => part).join("\n");
  const found = new Set();
  for (const m of rust.matchAll(/"((?:err|conv)\.[A-Za-z]+)"/g)) found.add(m[1]);
  assert.ok(found.size > 25, `poucas mensagens: ${found.size}`);
  for (const key of found) {
    if (key === "err.x") continue;
    assert.ok(keys.includes(key), key);
  }
});

test("sem texto em portugues fixo na interface", () => {
  const html = read("ui/index.html").replace(/<title>.*<\/title>/, "").replace(/<!--.*?-->/gs, "");
  const text = html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
  assert.strictEqual(text, "", `texto fixo no HTML: ${text}`);
  assert.ok(!/\s(placeholder|title|aria-label)="/.test(html), "atributo de texto fixo no HTML");
});
