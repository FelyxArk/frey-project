# Frey Project

App de Windows que fica na bandeja e abre com um atalho (`Ctrl+Alt+P`): cria a estrutura de pastas do seu projeto de vídeo e baixa os materiais do Google Drive e do YouTube para dentro dela. Tem também um conversor de vídeo.

## O que ele faz

- **Criar pastas e baixar:** escolha um modelo, dê um nome e cole os links. Cada link cai na pasta marcada do modelo, ou na que você escolher.
- **Só baixar:** baixa direto para uma pasta, sem criar estrutura.
- **Modelos de pastas:** crie, duplique e edite os seus.
- **Google Drive:** arquivos e pastas públicos, com subpastas e retomada de download.
- **YouTube:** 1080p e 720p em H.264 MP4, 4K para edição (baixa até 4K e converte para H.264 MP4), melhor qualidade ou só áudio.
- **Conversor:** MP4, MOV, MKV, WEBM, GIF, MP3, WAV e M4A, com GPU quando disponível.
- **Idiomas:** português, inglês, espanhol, francês, japonês, chinês simplificado, chinês tradicional, russo e árabe.

Nada é apagado nem sobrescrito: arquivos que já existem são pulados e avisados.

## Instalar

Baixe o instalador em [Releases](../../releases). Ele instala só para o seu usuário, sem pedir administrador, e já traz tudo o que o app precisa.

O instalador não tem assinatura digital. Se o Windows mostrar "editor desconhecido", clique em **Mais informações** e depois em **Executar assim mesmo**.

Requer Windows 10 ou 11 de 64 bits.

## Uso responsável

Baixe apenas material seu ou que você tenha permissão para baixar. Os termos do YouTube restringem downloads sem autorização, e os direitos autorais de cada vídeo continuam valendo. O app não usa a sua conta nem baixa vídeos protegidos por DRM.

## Problemas conhecidos

- O **X** e o **Esc** só escondem o painel.
- O **Sair** da bandeja remove o ícone, mas o processo pode continuar rodando até reiniciar o PC.
- O `yt-dlp` incluído não se atualiza sozinho. Se os downloads do YouTube pararem, instale a versão mais nova do Frey Project.
- As traduções ainda não tiveram revisão de falantes nativos.

## Desenvolvimento

Feito com [Tauri 2](https://tauri.app) e Rust. A interface é HTML, CSS e JavaScript em `ui/`.

```
cd src-tauri
cargo test
cargo build --release
```

O app procura `ffmpeg`, `ffprobe`, `yt-dlp` e `deno` primeiro na pasta `bin` ao lado do executável e depois no PATH.

Testes:

```
node --test tests/strings.test.cjs
powershell -ExecutionPolicy Bypass -File tests/Test-Settings.ps1
```

`Test-Settings.ps1` roda no app real com configuração temporária. Para incluir um download real, defina `FREY_TEST_DRIVE_FILE` com o link público de um arquivo pequeno.

Para gerar o instalador, com Inno Setup 6 e as ferramentas no PATH:

```
powershell -ExecutionPolicy Bypass -File packaging/Build-Installer.ps1 -WebView2Installer <caminho do instalador offline do WebView2>
```

## Créditos

O Frey Project só existe graças a estes projetos:

**Incluídos no instalador, sem alteração**

| Projeto | Para quê | Licença |
|---|---|---|
| [FFmpeg](https://github.com/FFmpeg/FFmpeg) | Conversão e junção de áudio e vídeo | GPL v3 nesta compilação |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | Downloads do YouTube | The Unlicense |
| [Deno](https://github.com/denoland/deno) | Runtime de JavaScript usado pelo yt-dlp | MIT |
| [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) | Interface do app, instalado só se faltar | Termos da Microsoft |

A compilação do FFmpeg é a [essentials de gyan.dev](https://www.gyan.dev/ffmpeg/builds/).

**Bibliotecas usadas no código**

| Projeto | Para quê | Licença |
|---|---|---|
| [Tauri](https://github.com/tauri-apps/tauri) | Base do app | MIT ou Apache 2.0 |
| [Plugins do Tauri](https://github.com/tauri-apps/plugins-workspace) | Atalho global, instância única, seleção de pastas e notificações | MIT ou Apache 2.0 |
| [ureq](https://github.com/algesten/ureq) | Downloads do Google Drive | MIT ou Apache 2.0 |
| [serde](https://github.com/serde-rs/serde) e [serde_json](https://github.com/serde-rs/json) | Leitura e gravação de configurações | MIT ou Apache 2.0 |
| [regex](https://github.com/rust-lang/regex) | Reconhecimento de links | MIT ou Apache 2.0 |
| [chrono](https://github.com/chronotope/chrono) | Datas nos nomes de pastas | MIT ou Apache 2.0 |

**Ferramentas e referências**

| Projeto | Para quê | Licença |
|---|---|---|
| [Inno Setup](https://github.com/jrsoftware/issrc) | Geração do instalador | Licença do Inno Setup |
| [gdown](https://github.com/wkentaro/gdown) | Referência para o fluxo de download do Google Drive, sem código copiado | MIT |

O instalador traz os avisos completos: `NOTICES.txt` e a pasta `licenses`, com a licença de cada biblioteca Rust incluída.

## Licença

O código do Frey Project está sob a [licença MIT](LICENSE). Os programas de terceiros seguem as próprias licenças.
