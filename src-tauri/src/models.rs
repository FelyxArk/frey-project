//! Modelos de pastas do usuario (mesmo formato do Frey Project Create) e criacao da estrutura no disco.
//! Nunca apaga nem sobrescreve o que ja esta no disco.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub root: String,
    pub folders: Vec<String>,
    /// Pasta do modelo que recebe os arquivos baixados; vazio e a raiz do projeto.
    pub destination: String,
    pub files: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub default_name: String,
    pub open_at_end: bool,
    pub templates: Vec<Template>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub root: String,
    pub folders: Vec<String>,
    pub files: Vec<(String, String)>,
}

/// O PowerShell grava lista de um item como texto simples; aceita os dois formatos.
fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn file_entries(value: Option<&Value>) -> Vec<(String, String)> {
    let items = match value {
        Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let path = item.get("caminho")?.as_str()?.to_string();
            Some((path, strings(item.get("conteudo")).join("\r\n")))
        })
        .collect()
}

pub fn parse_config(raw: &str) -> Result<Config, String> {
    let json: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|e| crate::i18n::code("err.configBad", &[&e.to_string()]))?;
    let list = match json.get("modelos") {
        Some(Value::Array(items)) => items.clone(),
        Some(item @ Value::Object(_)) => vec![item.clone()],
        _ => Vec::new(),
    };
    let templates: Vec<Template> = list
        .iter()
        .filter_map(|m| {
            let folders = strings(m.get("pastas"));
            // Arquivo antigo, sem a escolha salva: sugere a pasta de footage/brutos.
            let destination = match m.get("pastaDestino").and_then(Value::as_str) {
                Some(chosen) => chosen.to_string(),
                None => default_folder(&folders),
            };
            Some(Template {
                name: m.get("nome")?.as_str()?.to_string(),
                root: m.get("pastaRaiz").and_then(Value::as_str).unwrap_or("").to_string(),
                folders,
                destination,
                files: file_entries(m.get("arquivos")),
            })
        })
        .collect();
    if templates.is_empty() {
        return Err("err.modelsEmpty".into());
    }
    Ok(Config {
        default_name: json.get("nomePadrao").and_then(Value::as_str).unwrap_or("").to_string(),
        open_at_end: json.get("abrirAoFinal").and_then(Value::as_bool).unwrap_or(true),
        templates,
    })
}

/// Modelo que vem com o app. Nomes de modelo e de pasta sao dados do usuario: nao sao traduzidos.
pub const DEFAULT_MODEL: &str = "Comissões";

pub fn default_template() -> Template {
    Template {
        name: DEFAULT_MODEL.into(),
        root: "{data}_{nome}".into(),
        folders: ["01 Footage", "02 Insert", "03 Projetos", "04 Render", "05 SFX", "06 Musicas", "07 Legendas"].map(String::from).to_vec(),
        destination: "01 Footage".into(),
        files: Vec::new(),
    }
}

pub fn default_config() -> Config {
    Config { default_name: String::new(), open_at_end: true, templates: vec![default_template()] }
}

/// Sem arquivo salvo ainda, vale o padrao. Arquivo com erro nao e trocado em silencio.
pub fn load_config(path: &Path) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(raw) => parse_config(&raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default_config()),
        Err(e) => Err(crate::i18n::code("err.configBad", &[&e.to_string()])),
    }
}

pub fn to_json(config: &Config) -> String {
    let models: Vec<Value> = config
        .templates
        .iter()
        .map(|t| {
            let files: Vec<Value> = t
                .files
                .iter()
                .map(|(path, content)| json!({ "caminho": path, "conteudo": content.split("\r\n").collect::<Vec<_>>() }))
                .collect();
            json!({ "nome": t.name, "pastaRaiz": t.root, "pastas": t.folders, "pastaDestino": t.destination, "arquivos": files })
        })
        .collect();
    let value = json!({ "nomePadrao": config.default_name, "abrirAoFinal": config.open_at_end, "modelos": models });
    serde_json::to_string_pretty(&value).expect("json")
}

/// Grava em arquivo temporario e troca: uma queda no meio nao corrompe os modelos.
pub fn save_config(path: &Path, config: &Config) -> Result<(), String> {
    let fail = |e: std::io::Error| crate::i18n::code("err.saveFailed", &[&e.to_string()]);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, to_json(config)).map_err(fail)?;
    fs::rename(&temp, path).map_err(fail)
}

/// Limpa e valida o que veio do editor. Nome vazio ou repetido e recusado, nao corrigido.
pub fn normalize_templates(input: Vec<Template>) -> Result<Vec<Template>, String> {
    if input.is_empty() {
        return Err("err.modelsEmpty".into());
    }
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(input.len());
    for mut template in input {
        template.name = template.name.trim().to_string();
        if template.name.is_empty() {
            return Err("err.modelNameEmpty".into());
        }
        let key = template.name.to_lowercase();
        if seen.contains(&key) {
            return Err(crate::i18n::code("err.modelNameDup", &[&template.name]));
        }
        seen.push(key);
        template.root = template.root.trim().to_string();
        template.folders = template.folders.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
        template.destination = template.destination.trim().to_string();
        if !template.folders.contains(&template.destination) {
            template.destination = String::new();
        }
        out.push(template);
    }
    Ok(out)
}

pub fn clean_segment(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    replaced.trim().trim_end_matches(['.', ' ']).to_string()
}

fn expand(text: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = text.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// "02 Insert/Logos" vira "02 Insert\Logos"; ".." e segmentos vazios somem.
fn resolve_rel(rel: &str, vars: &HashMap<&str, String>) -> String {
    expand(rel, vars)
        .split(['/', '\\'])
        .map(clean_segment)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}

fn vars(name: &str) -> HashMap<&'static str, String> {
    let now = chrono::Local::now();
    HashMap::from([
        ("nome", clean_segment(name)),
        ("data", now.format("%Y-%m-%d").to_string()),
        ("ano", now.format("%Y").to_string()),
        ("mes", now.format("%m").to_string()),
        ("dia", now.format("%d").to_string()),
        ("hora", now.format("%H-%M").to_string()),
    ])
}

pub fn needs_name(t: &Template) -> bool {
    t.root.contains("{nome}")
        || t.folders.iter().any(|f| f.contains("{nome}"))
        || t.files.iter().any(|(path, _)| path.contains("{nome}"))
}

pub fn build_plan(t: &Template, name: &str) -> Plan {
    let vars = vars(name);
    let mut folders = Vec::new();
    for f in &t.folders {
        let rel = resolve_rel(f, &vars);
        if !rel.is_empty() && !folders.contains(&rel) {
            folders.push(rel);
        }
    }
    let files = t
        .files
        .iter()
        .map(|(path, content)| (resolve_rel(path, &vars), expand(content, &vars)))
        .filter(|(path, _)| !path.is_empty())
        .collect();
    Plan { root: resolve_rel(&t.root, &vars), folders, files }
}

/// Todas as pastas do plano, incluindo as pastas-mae ("a\b" gera "a" e "a\b").
pub fn target_folders(plan: &Plan) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for rel in &plan.folders {
        let mut current = String::new();
        for seg in rel.split('\\') {
            if !current.is_empty() {
                current.push('\\');
            }
            current.push_str(seg);
            if !seen.contains(&current) {
                seen.push(current.clone());
            }
        }
    }
    seen
}

/// Pasta que recebe os links novos, como aparece na lista do projeto; vazio e a raiz.
pub fn destination(template: &Template, plan: &Plan, name: &str) -> String {
    let chosen = resolve_rel(&template.destination, &vars(name));
    if target_folders(plan).contains(&chosen) { chosen } else { String::new() }
}

/// Sugestao para modelos antigos, sem escolha salva: a de footage/brutos; sem ela, a raiz (texto vazio).
pub fn default_folder(folders: &[String]) -> String {
    for pattern in ["footage", "bruto"] {
        if let Some(found) = folders.iter().find(|f| f.to_lowercase().contains(pattern)) {
            return found.clone();
        }
    }
    String::new()
}

pub fn project_base(plan: &Plan, dest: &Path) -> PathBuf {
    if plan.root.is_empty() { dest.to_path_buf() } else { dest.join(&plan.root) }
}

pub fn create_on_disk(plan: &Plan, dest: &Path) -> std::io::Result<PathBuf> {
    let base = project_base(plan, dest);
    fs::create_dir_all(&base)?;
    for rel in &plan.folders {
        fs::create_dir_all(base.join(rel))?;
    }
    for (rel, content) in &plan.files {
        let full = base.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        if !full.exists() {
            fs::write(&full, content)?;
        }
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}{ \"nomePadrao\": \"Novo\", \"abrirAoFinal\": false, \"modelos\": [
        { \"nome\": \"Comissões\", \"pastaRaiz\": \"{nome}\", \"pastas\": [\"01 Footage\", \"02 Insert/Logos\", \"02 Insert/Logos\"],
          \"arquivos\": [ { \"caminho\": \"00_briefing.txt\", \"conteudo\": [\"Projeto: {nome}\", \"fim\"] } ] },
        { \"nome\": \"Um so\", \"pastaRaiz\": \"\", \"pastas\": \"Brutos\" },
        { \"nome\": \"Vazio\", \"pastaRaiz\": \"{data}_{nome}\", \"pastas\": [] },
        { \"sem\": \"nome\" }
    ] }";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("frey-project-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn le_config_com_bom_e_formatos_do_powershell() {
        let cfg = parse_config(SAMPLE).unwrap();
        assert_eq!(cfg.templates.len(), 3);
        assert!(!cfg.open_at_end);
        assert_eq!(cfg.templates[1].folders, vec!["Brutos"]);
        assert!(parse_config("{ nao e json").is_err());
        assert!(parse_config("{\"modelos\": []}").is_err());
    }

    #[test]
    fn plano_limpa_nomes_e_nao_sobe_de_pasta() {
        let t = Template { name: "x".into(), root: "..\\{nome}".into(), folders: vec!["a/../b".into()], destination: String::new(), files: vec![] };
        let plan = build_plan(&t, "x:y");
        assert_eq!(plan.root, "x_y");
        assert_eq!(plan.folders, vec!["a\\b"]);
    }

    #[test]
    fn plano_expande_variaveis_e_remove_duplicatas() {
        let cfg = parse_config(SAMPLE).unwrap();
        let plan = build_plan(&cfg.templates[0], "Cliente");
        assert_eq!(plan.root, "Cliente");
        assert_eq!(plan.folders, vec!["01 Footage", "02 Insert\\Logos"]);
        assert_eq!(plan.files[0].1, "Projeto: Cliente\r\nfim");
        assert_eq!(target_folders(&plan), vec!["01 Footage", "02 Insert", "02 Insert\\Logos"]);
        assert!(needs_name(&cfg.templates[0]));
        assert!(!needs_name(&cfg.templates[1]));
    }

    #[test]
    fn padrao_e_o_modelo_comissoes_e_arquivo_novo_nao_e_erro() {
        let dir = temp_dir("default");
        let cfg = load_config(&dir.join("nao-existe.json")).unwrap();
        assert_eq!(cfg.templates.len(), 1);
        let t = &cfg.templates[0];
        assert_eq!(t.name, DEFAULT_MODEL);
        assert_eq!(t.root, "{data}_{nome}");
        assert_eq!(t.folders, ["01 Footage", "02 Insert", "03 Projetos", "04 Render", "05 SFX", "06 Musicas", "07 Legendas"]);
    }

    #[test]
    fn salvar_e_ler_preserva_tudo_incluindo_arquivos() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("sub").join("modelos.json");
        let mut cfg = parse_config(SAMPLE).unwrap();
        cfg.templates.truncate(1);
        save_config(&path, &cfg).unwrap();
        let back = load_config(&path).unwrap();
        assert_eq!(back.templates[0].folders, cfg.templates[0].folders);
        assert_eq!(back.templates[0].files, cfg.templates[0].files);
        assert_eq!(back.open_at_end, cfg.open_at_end);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn arquivo_com_erro_nao_vira_padrao_em_silencio() {
        let dir = temp_dir("broken");
        let path = dir.join("modelos.json");
        fs::write(&path, "{ quebrado").unwrap();
        assert!(load_config(&path).unwrap_err().starts_with("err.configBad"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ quebrado");
    }

    #[test]
    fn editor_recusa_nome_vazio_ou_repetido_e_limpa_pastas() {
        let t = |name: &str, folders: &[&str]| Template { name: name.into(), root: " {nome} ".into(), folders: folders.iter().map(|s| s.to_string()).collect(), destination: String::new(), files: vec![] };
        assert_eq!(normalize_templates(vec![]).unwrap_err(), "err.modelsEmpty");
        assert_eq!(normalize_templates(vec![t("  ", &[])]).unwrap_err(), "err.modelNameEmpty");
        assert!(normalize_templates(vec![t("Ação", &[]), t("AÇÃO", &[])]).unwrap_err().starts_with("err.modelNameDup"));
        let ok = normalize_templates(vec![t(" A ", &["  x ", "", "   ", "y/z"])]).unwrap();
        assert_eq!((ok[0].name.as_str(), ok[0].root.as_str()), ("A", "{nome}"));
        assert_eq!(ok[0].folders, ["x", "y/z"]);
    }

    #[test]
    fn destino_e_salvo_e_sobrevive_a_leitura() {
        let dir = temp_dir("destination");
        let path = dir.join("modelos.json");
        let mut cfg = default_config();
        cfg.templates[0].destination = "02 Insert".into();
        save_config(&path, &cfg).unwrap();
        assert_eq!(load_config(&path).unwrap().templates[0].destination, "02 Insert");
    }

    #[test]
    fn padrao_recebe_em_footage_e_arquivo_antigo_ganha_sugestao() {
        assert_eq!(default_template().destination, "01 Footage");
        let old = parse_config(SAMPLE).unwrap();
        assert_eq!(old.templates[0].destination, "01 Footage");
        assert_eq!(old.templates[3 - 1].destination, "");
        let antigo = parse_config("{\"modelos\":[{\"nome\":\"A\",\"pastas\":[\"x\",\"01 Footage\"]}]}").unwrap();
        assert_eq!(antigo.templates[0].destination, "01 Footage");
        let escolhido = parse_config("{\"modelos\":[{\"nome\":\"A\",\"pastas\":[\"x\",\"01 Footage\"],\"pastaDestino\":\"x\"}]}").unwrap();
        assert_eq!(escolhido.templates[0].destination, "x");
    }

    #[test]
    fn destino_aponta_para_a_pasta_do_plano_ou_para_a_raiz() {
        let mut t = Template { name: "x".into(), root: "{nome}".into(), folders: vec!["02 Insert/Logos".into(), "{nome} final".into()], destination: "02 Insert/Logos".into(), files: vec![] };
        let plan = build_plan(&t, "Cliente");
        assert_eq!(destination(&t, &plan, "Cliente"), "02 Insert\\Logos");
        t.destination = "{nome} final".into();
        assert_eq!(destination(&t, &plan, "Cliente"), "Cliente final");
        t.destination = "nao existe".into();
        assert_eq!(destination(&t, &plan, "Cliente"), "");
    }

    #[test]
    fn editor_limpa_destino_que_nao_e_pasta_do_modelo() {
        let t = |d: &str| Template { name: "A".into(), root: "".into(), folders: vec![" x ".into()], destination: d.into(), files: vec![] };
        assert_eq!(normalize_templates(vec![t(" x ")]).unwrap()[0].destination, "x");
        assert_eq!(normalize_templates(vec![t("sumiu")]).unwrap()[0].destination, "");
    }

    #[test]
    fn pasta_padrao_e_a_de_footage() {
        assert_eq!(default_folder(&["02 Insert".into(), "01 Footage".into()]), "01 Footage");
        assert_eq!(default_folder(&["Brutos".into()]), "Brutos");
        assert_eq!(default_folder(&["Audio".into()]), "");
    }

    #[test]
    fn criar_nao_sobrescreve_arquivo_existente() {
        let dir = temp_dir("create");
        let cfg = parse_config(SAMPLE).unwrap();
        let plan = build_plan(&cfg.templates[0], "Cliente");
        let base = create_on_disk(&plan, &dir).unwrap();
        assert!(base.join("02 Insert\\Logos").is_dir());
        fs::write(base.join("00_briefing.txt"), "editado").unwrap();
        create_on_disk(&plan, &dir).unwrap();
        assert_eq!(fs::read_to_string(base.join("00_briefing.txt")).unwrap(), "editado");
        fs::remove_dir_all(&dir).unwrap();
    }
}
