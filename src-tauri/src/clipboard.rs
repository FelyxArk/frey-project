//! Deixa pronto o link recem-copiado sem vigiar o conteudo da area de transferencia.
//! Em segundo plano so se pergunta ao Windows se houve uma copia nova (um numero, sem texto).
//! O texto e lido uma vez, ao abrir o painel, e so se a copia for nova e recente.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Copia mais velha que isto nao e mais "o link que acabei de copiar".
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);
const MAX_TEXT: usize = 20_000;

#[link(name = "user32")]
extern "system" {
    fn GetClipboardSequenceNumber() -> u32;
}

fn current_sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

#[derive(Default)]
pub struct Watch {
    /// Copia ja considerada; comeca na que existia quando o app abriu.
    handled: AtomicU32,
    latest: Mutex<(u32, Option<Instant>)>,
}

impl Watch {
    /// O que ja estava copiado antes de o app abrir nunca entra.
    pub fn start(&self) {
        let now = current_sequence();
        self.handled.store(now, Ordering::SeqCst);
        *self.latest.lock().unwrap() = (now, None);
    }

    /// Chamado a cada segundo: anota so quando houve copia nova.
    pub fn tick(&self) {
        let now = current_sequence();
        let mut latest = self.latest.lock().unwrap();
        if latest.0 != now {
            *latest = (now, Some(Instant::now()));
        }
    }

    /// Consome a copia atual. Devolve true se vale ler o texto, uma unica vez por copia.
    pub fn take_new(&self) -> bool {
        self.tick();
        let (sequence, changed_at) = *self.latest.lock().unwrap();
        let handled = self.handled.swap(sequence, Ordering::SeqCst);
        is_fresh(sequence, handled, changed_at.map(|at| at.elapsed()))
    }
}

pub fn is_fresh(sequence: u32, handled: u32, age: Option<Duration>) -> bool {
    sequence != handled && age.is_some_and(|age| age <= MAX_AGE)
}

/// So enderecos de verdade: codigos soltos, que o colar manual aceita como ID,
/// poderiam ser qualquer coisa copiada, como uma senha.
pub fn links_in(text: &str) -> Vec<crate::drive::Link> {
    if text.len() > MAX_TEXT {
        return Vec::new();
    }
    crate::drive::parse_links(text).into_iter().filter(|link| link.url.starts_with("http")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_copia_nova_e_recente_conta() {
        let recent = Some(Duration::from_secs(5));
        assert!(is_fresh(2, 1, recent));
        assert!(!is_fresh(2, 2, recent), "a mesma copia nao conta duas vezes");
        assert!(!is_fresh(2, 1, None), "copia sem hora conhecida e de antes do app abrir");
        assert!(!is_fresh(2, 1, Some(MAX_AGE + Duration::from_secs(1))), "copia velha nao volta");
        assert!(is_fresh(2, 1, Some(MAX_AGE)));
    }

    #[test]
    fn copia_que_ja_existia_quando_o_app_abriu_nao_entra() {
        let watch = Watch::default();
        watch.start();
        assert!(!watch.take_new());
    }

    #[test]
    fn so_entram_enderecos_do_drive_e_do_youtube() {
        let id = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert!(links_in(id).is_empty(), "codigo solto pode ser senha");
        assert!(links_in("texto qualquer https://example.com/x").is_empty());
        assert!(links_in(&"x".repeat(MAX_TEXT + 1)).is_empty());
        let links = links_in("veja https://youtu.be/abc e https://drive.google.com/drive/folders/def");
        assert_eq!(links.iter().map(|l| l.kind).collect::<Vec<_>>(), ["youtube", "folder"]);
    }
}
