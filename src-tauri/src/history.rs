//! Historico dos downloads: o que terminou, falhou ou foi cancelado, para a aba Baixados.
//! Fica num JSON nas configuracoes; os mais novos primeiro, com limite para nao crescer sem fim.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LIMIT: usize = 500;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    pub id: String,
    pub url: String,
    /// youtube, x, instagram, tiktok, file ou folder (Drive).
    pub kind: String,
    pub title: String,
    pub thumb: String,
    /// Pasta onde os arquivos foram salvos.
    pub folder: String,
    pub files: Vec<String>,
    /// done, warning, fail ou cancelled.
    pub state: String,
    pub message: String,
    pub quality: String,
    pub bytes: u64,
    /// Segundos desde 1970.
    pub when: u64,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn load(path: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(path: &Path, list: &[Entry]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(list) {
        // Grava num temporario e troca: um corte de energia nao deixa o historico pela metade.
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }
}

/// Novo no topo. Tentar de novo o mesmo link substitui a falha anterior em vez de repetir.
pub fn add(list: &mut Vec<Entry>, entry: Entry) {
    list.retain(|e| !(e.url == entry.url && e.folder == entry.folder && e.state != "done" && e.state != "warning"));
    list.insert(0, entry);
    list.truncate(LIMIT);
}

pub fn remove(list: &mut Vec<Entry>, id: &str) {
    list.retain(|e| e.id != id);
}

/// "Limpar concluidos": tira o que deu certo e deixa as falhas para tentar de novo.
pub fn clear_done(list: &mut Vec<Entry>) {
    list.retain(|e| e.state != "done" && e.state != "warning");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, url: &str, state: &str) -> Entry {
        Entry { id: id.into(), url: url.into(), folder: "D:\\p".into(), state: state.into(), ..Default::default() }
    }

    #[test]
    fn novo_no_topo_e_falha_repetida_nao_duplica() {
        let mut list = vec![entry("1", "a", "fail"), entry("2", "b", "done")];
        add(&mut list, entry("3", "a", "done"));
        assert_eq!(list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["3", "2"]);
        add(&mut list, entry("4", "b", "done"));
        assert_eq!(list.len(), 3, "baixar de novo um que deu certo fica registrado");
    }

    #[test]
    fn limpar_concluidos_deixa_as_falhas() {
        let mut list = vec![entry("1", "a", "fail"), entry("2", "b", "done"), entry("3", "c", "cancelled"), entry("4", "d", "warning")];
        clear_done(&mut list);
        assert_eq!(list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["1", "3"]);
        remove(&mut list, "1");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn grava_e_le_de_volta_e_respeita_o_limite() {
        let dir = std::env::temp_dir().join(format!("frey-hist-{}", std::process::id()));
        let path = dir.join("historico.json");
        let mut list = Vec::new();
        for i in 0..LIMIT + 20 {
            add(&mut list, entry(&i.to_string(), &format!("u{i}"), "done"));
        }
        assert_eq!(list.len(), LIMIT);
        save(&path, &list);
        assert_eq!(load(&path), list);
        assert!(load(&dir.join("nao-existe.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
