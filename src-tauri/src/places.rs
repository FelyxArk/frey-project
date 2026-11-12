//! Atalhos para o destino: caminhos usados por ultimo e as subpastas da pasta escolhida.
//! So le pastas; nada aqui cria, move ou apaga.

use serde::Serialize;
use std::path::Path;

/// Quantos caminhos recentes ficam guardados.
pub const HISTORY_SIZE: usize = 8;
/// Pasta com milhares de subpastas nao trava o painel: a lista para aqui.
pub const SUBFOLDER_LIMIT: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Place {
    pub name: String,
    pub path: String,
}

fn same_path(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) {
        trim(a).to_lowercase() == trim(b).to_lowercase()
    } else {
        trim(a) == trim(b)
    }
}

/// O caminho usado vai para o topo; repetido sai de onde estava; o mais antigo cai.
pub fn remember(history: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    history.retain(|p| !same_path(p, path));
    history.insert(0, path.to_string());
    history.truncate(HISTORY_SIZE);
}

fn place(path: &Path) -> Place {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
    Place { name, path: path.display().to_string() }
}

/// Recentes que ainda existem, do mais novo para o mais antigo.
pub fn recent(history: &[String]) -> Vec<Place> {
    history.iter().map(Path::new).filter(|p| p.is_dir()).map(place).collect()
}

#[cfg(windows)]
fn hidden(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    entry.metadata().map(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0).unwrap_or(false)
}

#[cfg(not(windows))]
fn hidden(_entry: &std::fs::DirEntry) -> bool {
    false
}

/// Ordem de explorador: sem diferenciar maiusculas, e "2" antes de "10".
fn natural_key(name: &str) -> Vec<(u8, String)> {
    let mut parts = Vec::new();
    let mut chars = name.chars().peekable();
    while let Some(&ch) = chars.peek() {
        let digit = ch.is_ascii_digit();
        let mut part = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() != digit {
                break;
            }
            part.push(c);
            chars.next();
        }
        if digit {
            let trimmed = part.trim_start_matches('0');
            parts.push((0, format!("{:0>20}", trimmed)));
        } else {
            parts.push((1, part.to_lowercase()));
        }
    }
    parts
}

/// Subpastas diretas da pasta escolhida, sem as ocultas e as do sistema.
pub fn subfolders(root: &Path) -> Vec<Place> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut list: Vec<Place> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && !hidden(e))
        .map(|e| place(&e.path()))
        .collect();
    list.sort_by_cached_key(|p| natural_key(&p.name));
    list.truncate(SUBFOLDER_LIMIT);
    list
}

/// Tira um caminho do historico (o X da lista de recentes).
pub fn forget(history: &mut Vec<String>, path: &str) {
    history.retain(|p| !same_path(p, path));
}

/// Nome de subpasta vindo do painel: so um nivel, existente, dentro da pasta escolhida.
pub fn subfolder(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let plain = !name.is_empty() && name != "." && name != ".." && !name.contains(['\\', '/', ':']);
    let path = root.join(name);
    (plain && path.is_dir()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("frey-places-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn historico_poe_o_ultimo_no_topo_sem_repetir_e_guarda_oito() {
        let mut h = Vec::new();
        for i in 1..=10 {
            remember(&mut h, &format!("C:\\p{i}"));
        }
        assert_eq!(h.len(), HISTORY_SIZE);
        assert_eq!(h[0], "C:\\p10");
        assert_eq!(h[7], "C:\\p3");
        remember(&mut h, "C:\\p5");
        assert_eq!(h[0], "C:\\p5");
        assert_eq!(h.iter().filter(|p| p.as_str() == "C:\\p5").count(), 1);
        assert_eq!(h.len(), HISTORY_SIZE);
        remember(&mut h, "  ");
        assert_eq!(h[0], "C:\\p5");
    }

    #[cfg(windows)]
    #[test]
    fn no_windows_o_mesmo_caminho_com_outra_caixa_ou_barra_final_nao_duplica() {
        let mut h = vec!["D:\\Clientes\\Ana".to_string()];
        remember(&mut h, "d:\\clientes\\ana\\");
        assert_eq!(h, vec!["d:\\clientes\\ana\\".to_string()]);
    }

    #[test]
    fn recentes_que_sumiram_nao_aparecem() {
        let dir = temp("recent");
        let a = dir.join("Ana");
        std::fs::create_dir_all(&a).unwrap();
        let gone = dir.join("Sumiu").display().to_string();
        let list = recent(&[gone, a.display().to_string()]);
        assert_eq!(list, vec![Place { name: "Ana".into(), path: a.display().to_string() }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subpastas_em_ordem_natural_sem_arquivos_nem_ocultas() {
        let dir = temp("subs");
        for name in ["cliente 10", "Cliente 2", "beta", ".git", "Alfa"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        std::fs::write(dir.join("nota.txt"), "x").unwrap();
        let names: Vec<String> = subfolders(&dir).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Alfa", "beta", "Cliente 2", "cliente 10"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subpasta_pedida_pelo_painel_so_vale_um_nivel_e_existente() {
        let dir = temp("sub");
        std::fs::create_dir_all(dir.join("Ana").join("Fundo")).unwrap();
        assert_eq!(subfolder(&dir, "Ana"), Some(dir.join("Ana")));
        for bad in ["", ".", "..", "Ana\\Fundo", "Ana/Fundo", "C:", "Sumiu", "..\\Ana"] {
            assert_eq!(subfolder(&dir, bad), None, "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn esquecer_tira_so_o_caminho_pedido() {
        let mut h = vec!["C:\\a".to_string(), "C:\\b".to_string()];
        forget(&mut h, "C:\\a");
        assert_eq!(h, vec!["C:\\b".to_string()]);
    }
}
