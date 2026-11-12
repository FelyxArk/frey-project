//! O que muda entre Windows e Mac ao abrir programas externos (ffmpeg, ffprobe, yt-dlp).

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Nome do executavel no sistema atual: "ffmpeg.exe" no Windows, "ffmpeg" no Mac.
pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Procura um programa no PATH.
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let file = exe_name(name);
    std::env::split_paths(&paths).map(|dir| dir.join(&file)).find(|p| p.is_file())
}

/// Sem janela de console no Windows; no Mac nao ha o que esconder.
pub fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.stdin(Stdio::null())
}

/// No Mac o programa ganha um grupo de processos proprio: cancelar encerra tambem
/// o ffmpeg que o yt-dlp abre. No Windows o taskkill /T ja cuida disso.
pub fn own_group(command: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

/// Encerra o processo e os filhos dele.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = quiet(&mut Command::new("taskkill"))
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    unsafe {
        // Grupo criado em own_group: o numero do grupo e o do processo.
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
