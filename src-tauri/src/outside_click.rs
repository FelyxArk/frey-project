//! Clique duplo fora do painel: esconde. Enquanto o painel esta aberto, olha o estado do botao
//! do mouse algumas vezes por segundo. Nao instala gancho global e nao interfere no clique.
//! No Mac tambem nao usa monitor global de eventos: so le se o botao esta pressionado.

use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Limits {
    pub time: Duration,
    pub width: i32,
    pub height: i32,
}

#[derive(Default)]
pub struct DoubleClick {
    last: Option<(Instant, i32, i32)>,
}

impl DoubleClick {
    /// Registra um clique. Devolve true quando ele completa um clique duplo fora do painel.
    pub fn press(&mut self, now: Instant, x: i32, y: i32, inside: bool, limits: Limits) -> bool {
        if inside {
            self.last = None;
            return false;
        }
        let double = self.last.is_some_and(|(at, last_x, last_y)| {
            now.duration_since(at) <= limits.time && (x - last_x).abs() <= limits.width / 2 && (y - last_y).abs() <= limits.height / 2
        });
        self.last = if double { None } else { Some((now, x, y)) };
        double
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetDoubleClickTime() -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetWindowRect(window: *mut std::ffi::c_void, rect: *mut Rect) -> i32;
    fn IsWindowVisible(window: *mut std::ffi::c_void) -> i32;
}

/// `busy` diz para ignorar os cliques, como quando uma caixa de dialogo do app esta aberta.
#[cfg(windows)]
pub fn watch(window: isize, busy: impl Fn() -> bool + Send + 'static, close: impl Fn() + Send + 'static) {
    const VK_LBUTTON: i32 = 0x01;
    const VK_RBUTTON: i32 = 0x02;
    const SM_SWAPBUTTON: i32 = 23;
    const SM_CXDOUBLECLK: i32 = 36;
    const SM_CYDOUBLECLK: i32 = 37;
    std::thread::spawn(move || {
        let handle = window as *mut std::ffi::c_void;
        let mut clicks = DoubleClick::default();
        let mut was_down = false;
        let mut watching = false;
        loop {
            if unsafe { IsWindowVisible(handle) } == 0 {
                watching = false;
                std::thread::sleep(Duration::from_millis(120));
                continue;
            }
            std::thread::sleep(Duration::from_millis(15));
            let key = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
            let down = unsafe { GetAsyncKeyState(key) } < 0;
            if !watching {
                // Um clique que ja estava em andamento ao abrir nao conta.
                watching = true;
                was_down = down;
                clicks.reset();
                continue;
            }
            let pressed = down && !was_down;
            was_down = down;
            if !pressed {
                continue;
            }
            if busy() {
                clicks.reset();
                continue;
            }
            let (mut cursor, mut rect) = (Point::default(), Rect::default());
            if unsafe { GetCursorPos(&mut cursor) == 0 || GetWindowRect(handle, &mut rect) == 0 } {
                clicks.reset();
                continue;
            }
            let inside = cursor.x >= rect.left && cursor.x < rect.right && cursor.y >= rect.top && cursor.y < rect.bottom;
            let limits = unsafe {
                Limits {
                    time: Duration::from_millis(GetDoubleClickTime() as u64),
                    width: GetSystemMetrics(SM_CXDOUBLECLK),
                    height: GetSystemMetrics(SM_CYDOUBLECLK),
                }
            };
            if clicks.press(Instant::now(), cursor.x, cursor.y, inside, limits) {
                close();
            }
        }
    });
}

/// Mac: o mesmo controle, com o estado do botao lido do AppKit e a posicao lida do Tauri.
/// So pergunta onde esta a janela quando ha um clique: o resto do tempo e um numero por leitura.
#[cfg(target_os = "macos")]
pub fn watch(window: tauri::WebviewWindow, busy: impl Fn() -> bool + Send + 'static, close: impl Fn() + Send + 'static) {
    use objc2::{class, msg_send};
    // Distancia maxima entre os dois cliques, em pixels de tela.
    const SLOP: i32 = 8;
    std::thread::spawn(move || {
        let mut clicks = DoubleClick::default();
        let mut was_down = false;
        let mut watching = false;
        loop {
            if !window.is_visible().unwrap_or(false) {
                watching = false;
                std::thread::sleep(Duration::from_millis(120));
                continue;
            }
            std::thread::sleep(Duration::from_millis(15));
            let buttons: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
            let down = buttons & 1 != 0;
            if !watching {
                // Um clique que ja estava em andamento ao abrir nao conta.
                watching = true;
                was_down = down;
                clicks.reset();
                continue;
            }
            let pressed = down && !was_down;
            was_down = down;
            if !pressed {
                continue;
            }
            if busy() {
                clicks.reset();
                continue;
            }
            let (Ok(cursor), Ok(position), Ok(size)) = (window.cursor_position(), window.outer_position(), window.outer_size()) else {
                clicks.reset();
                continue;
            };
            let (x, y) = (cursor.x as i32, cursor.y as i32);
            let inside = x >= position.x && x < position.x + size.width as i32 && y >= position.y && y < position.y + size.height as i32;
            let seconds: f64 = unsafe { msg_send![class!(NSEvent), doubleClickInterval] };
            let limits = Limits { time: Duration::from_secs_f64(seconds.clamp(0.2, 2.0)), width: SLOP * 2, height: SLOP * 2 };
            if clicks.press(Instant::now(), x, y, inside, limits) {
                close();
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { time: Duration::from_millis(500), width: 4, height: 4 };

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn um_clique_fora_nao_fecha() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(Instant::now(), 10, 10, false, LIMITS));
    }

    #[test]
    fn dois_cliques_rapidos_no_mesmo_ponto_fecham() {
        let (mut clicks, start) = (DoubleClick::default(), Instant::now());
        assert!(!clicks.press(start, 10, 10, false, LIMITS));
        assert!(clicks.press(after(start, 200), 11, 9, false, LIMITS));
    }

    #[test]
    fn cliques_lentos_nao_fecham() {
        let (mut clicks, start) = (DoubleClick::default(), Instant::now());
        assert!(!clicks.press(start, 10, 10, false, LIMITS));
        assert!(!clicks.press(after(start, 501), 10, 10, false, LIMITS));
    }

    #[test]
    fn cliques_em_pontos_distantes_nao_fecham() {
        let (mut clicks, start) = (DoubleClick::default(), Instant::now());
        assert!(!clicks.press(start, 10, 10, false, LIMITS));
        assert!(!clicks.press(after(start, 100), 300, 10, false, LIMITS));
    }

    #[test]
    fn clique_dentro_do_painel_nao_conta() {
        let (mut clicks, start) = (DoubleClick::default(), Instant::now());
        assert!(!clicks.press(start, 10, 10, true, LIMITS));
        assert!(!clicks.press(after(start, 100), 10, 10, true, LIMITS));
        assert!(!clicks.press(after(start, 200), 10, 10, false, LIMITS));
        assert!(!clicks.press(after(start, 300), 10, 10, true, LIMITS));
        assert!(!clicks.press(after(start, 400), 10, 10, false, LIMITS));
    }

    #[test]
    fn terceiro_clique_comeca_outra_contagem() {
        let (mut clicks, start) = (DoubleClick::default(), Instant::now());
        assert!(!clicks.press(start, 10, 10, false, LIMITS));
        assert!(clicks.press(after(start, 100), 10, 10, false, LIMITS));
        assert!(!clicks.press(after(start, 200), 10, 10, false, LIMITS));
    }
}
