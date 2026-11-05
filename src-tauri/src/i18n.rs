//! Idiomas do produto (os mesmos do Frey). O painel traduz em ui/strings.js; aqui ficam so
//! os textos que o proprio Windows mostra: bandeja, janelas de selecao e notificacoes.

pub const LANGUAGES: [&str; 9] = ["pt", "en", "es", "fr", "ja", "zh", "zht", "ru", "ar"];

/// Mensagem para o painel traduzir: chave e argumentos separados por um caractere de controle.
pub fn code(key: &str, args: &[&str]) -> String {
    let mut out = key.to_string();
    for arg in args {
        out.push('\u{1f}');
        out.push_str(arg);
    }
    out
}

/// Ordem de cada linha: pt, en, es, fr, ja, zh, zht, ru, ar.
const NATIVE: [(&str, [&str; 9]); 10] = [
    ("trayOpen", ["Abrir painel", "Open panel", "Abrir panel", "Ouvrir le panneau", "パネルを開く", "打开面板", "開啟面板", "Открыть панель", "فتح اللوحة"]),
    ("trayQuit", ["Sair", "Quit", "Salir", "Quitter", "終了", "退出", "結束", "Выйти", "خروج"]),
    ("dialogDest", ["Onde criar o projeto?", "Where should the project be created?", "¿Dónde crear el proyecto?", "Où créer le projet ?", "プロジェクトの作成先", "在哪里创建项目？", "要在哪裡建立專案？", "Где создать проект?", "أين يتم إنشاء المشروع؟"]),
    ("dialogFiles", ["Escolher vídeos", "Choose videos", "Elegir vídeos", "Choisir des vidéos", "動画を選択", "选择视频", "選擇影片", "Выбрать видео", "اختيار مقاطع الفيديو"]),
    ("dialogOut", ["Salvar conversões em", "Save conversions in", "Guardar conversiones en", "Enregistrer les conversions dans", "変換ファイルの保存先", "转换文件保存到", "轉換檔案儲存至", "Сохранять результаты в", "حفظ الملفات المحولة في"]),
    ("filterMedia", ["Vídeos e áudios", "Videos and audio", "Vídeos y audios", "Vidéos et audio", "動画と音声", "视频和音频", "影片和音訊", "Видео и аудио", "الفيديو والصوت"]),
    ("filterAll", ["Todos os arquivos", "All files", "Todos los archivos", "Tous les fichiers", "すべてのファイル", "所有文件", "所有檔案", "Все файлы", "كل الملفات"]),
    ("notifyDownloaded", ["{0} link(s) baixado(s).", "{0} link(s) downloaded.", "{0} enlace(s) descargado(s).", "{0} lien(s) téléchargé(s).", "{0} 件のリンクをダウンロードしました。", "已下载 {0} 个链接。", "已下載 {0} 個連結。", "Загружено ссылок: {0}.", "تم تنزيل {0} من الروابط."]),
    ("notifyDownloadFailed", ["{0} link(s) baixado(s), {1} com falha. Abra o painel para tentar de novo.", "{0} link(s) downloaded, {1} failed. Open the panel to try again.", "{0} enlace(s) descargado(s), {1} con error. Abre el panel para reintentar.", "{0} lien(s) téléchargé(s), {1} en échec. Ouvrez le panneau pour réessayer.", "{0} 件ダウンロード、{1} 件失敗。パネルを開いて再試行してください。", "已下载 {0} 个，{1} 个失败。打开面板重试。", "已下載 {0} 個，{1} 個失敗。開啟面板重試。", "Загружено: {0}, с ошибкой: {1}. Откройте панель, чтобы повторить.", "تم تنزيل {0}، وفشل {1}. افتح اللوحة لإعادة المحاولة."]),
    ("notifyConverted", ["{0} convertido(s), {1} com falha.", "{0} converted, {1} failed.", "{0} convertido(s), {1} con error.", "{0} converti(s), {1} en échec.", "{0} 件変換、{1} 件失敗。", "已转换 {0} 个，{1} 个失败。", "已轉換 {0} 個，{1} 個失敗。", "Преобразовано: {0}, с ошибкой: {1}.", "تم تحويل {0}، وفشل {1}."]),
];

/// Texto nativo no idioma pedido; idioma ainda nao definido cai no portugues.
pub fn native(language: &str, key: &str) -> String {
    let index = LANGUAGES.iter().position(|l| *l == language).unwrap_or(0);
    NATIVE.iter().find(|(k, _)| *k == key).map(|(_, texts)| texts[index].to_string()).unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_idioma_tem_texto_e_mantem_os_marcadores() {
        for (key, texts) in NATIVE {
            for text in texts {
                assert!(!text.is_empty(), "{key}");
                for mark in ["{0}", "{1}"] {
                    assert_eq!(text.contains(mark), texts[0].contains(mark), "{key}: {text}");
                }
            }
        }
    }

    #[test]
    fn idioma_desconhecido_cai_no_portugues_e_mensagem_leva_argumentos() {
        assert_eq!(native("xx", "trayQuit"), "Sair");
        assert_eq!(native("ja", "trayQuit"), "終了");
        assert_eq!(code("err.x", &["a", "b"]), "err.x\u{1f}a\u{1f}b");
    }
}
