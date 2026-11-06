# Testa a aba Configuracoes no app real pelo depurador do WebView (CDP).
# Nao usa mouse nem teclado do Windows: nada pode cair em outra janela.
# Usa configuracao e destino temporarios (FREY_PROJECT_CONFIG_DIR): nunca toca nos dados do usuario.
# Recusa rodar com o app aberto: o Frey Project e instancia unica e o teste conversaria com o do usuario.
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File Test-Settings.ps1 [-OutDir pasta]
param([string]$OutDir = (Join-Path $env:TEMP "FreyProjectSettings-$([guid]::NewGuid().ToString('N'))"))
$ErrorActionPreference = 'Stop'
$exe = Join-Path $env:LOCALAPPDATA 'frey-project-target\release\frey-project.exe'
$dest = Join-Path $OutDir 'dest'
Remove-Item -LiteralPath $OutDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $dest | Out-Null
if (Get-Process frey-project -ErrorAction SilentlyContinue) { throw 'O Frey Project esta aberto. Feche pela bandeja (Sair) antes de testar.' }
$cfgDir = Join-Path $OutDir 'config'; $mj = "$cfgDir\modelos.json"; $sj = "$cfgDir\settings.json"
$env:FREY_PROJECT_CONFIG_DIR = $cfgDir
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333 --remote-allow-origins=*'

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class WinCheck {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
"@
function PanelWindow { [WinCheck]::FindWindow('Tauri Window', 'Frey Project') }
$results = New-Object System.Collections.ArrayList
$script:ws = $null; $script:lastName = $null
function Check([string]$l, [bool]$p) { [void]$results.Add(('{0}  {1}' -f $(if ($p) { 'PASS' } else { 'FAIL' }), $l)) }
function Send([string]$method, $params) {
  $id = Get-Random
  $msg = ConvertTo-Json -Compress -Depth 6 @{ id = $id; method = $method; params = $params }
  $seg = New-Object ArraySegment[byte] (,[Text.Encoding]::UTF8.GetBytes($msg))
  $script:ws.SendAsync($seg, 'Text', $true, [Threading.CancellationToken]::None).Wait()
  while ($true) {
    $buf = New-Object byte[] 4194304; $text = ''
    do { $r = $script:ws.ReceiveAsync((New-Object ArraySegment[byte] (,$buf)), [Threading.CancellationToken]::None); $r.Wait(); $text += [Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count) } while (-not $r.Result.EndOfMessage)
    $o = $text | ConvertFrom-Json
    if ($o.id -eq $id) { return $o.result }
  }
}
function Eval([string]$expr) { (Send 'Runtime.evaluate' @{ expression = $expr; returnByValue = $true }).result.value }
function Trace([string]$what) {
  $v = Eval 'document.getElementById("name").value'
  if ($v -ne $script:lastName) { [void]$results.Add("TRACE nome do projeto apos '$what': [$v]"); $script:lastName = $v }
}
function Click([string]$id) { [void](Eval "document.getElementById('$id').click()"); Start-Sleep -Milliseconds 500; Trace "clicar $id" }
# Digita pelo caminho real de entrada do navegador, substituindo o conteudo do campo.
function TypeIn([string]$id, [string]$text) {
  [void](Eval "var e=document.getElementById('$id'); e.focus(); e.select();")
  [void](Send 'Input.insertText' @{ text = $text })
  Start-Sleep -Milliseconds 700
  Trace "digitar em $id"
}
function Choose([string]$id, [string]$value) {
  [void](Eval "var e=document.getElementById('$id'); e.value=$(ConvertTo-Json $value); e.dispatchEvent(new Event('change',{bubbles:true}));")
  Start-Sleep -Milliseconds 600
  Trace "escolher em $id"
}
# Clica em "+ Adicionar pasta" e digita na linha nova, como o usuario.
function AddFolder([string]$text) {
  [void](Eval "document.getElementById('sAddFolder').click()")
  Start-Sleep -Milliseconds 150
  [void](Send 'Input.insertText' @{ text = $text })
  Start-Sleep -Milliseconds 600
  Trace "adicionar pasta"
}
function FolderRows { Eval 'JSON.stringify([...document.querySelectorAll("#sFolderList .folder-row")].map(l=>[l.querySelector("input[type=text]").value,l.querySelector("input[type=radio]").checked]))' | ConvertFrom-Json }
function FolderCount { [int](Eval 'document.querySelectorAll("#sFolderList .folder-row").length') }
function MarkFolder([int]$i) { [void](Eval "document.querySelectorAll('#sFolderList input[type=radio]')[$i].click()"); Start-Sleep -Milliseconds 600 }
function RemoveFolder([int]$i) { [void](Eval "document.querySelectorAll('#sFolderList button')[$i].click()"); Start-Sleep -Milliseconds 600 }
function RenameFolder([int]$i, [string]$text) {
  [void](Eval "var e=document.querySelectorAll('#sFolderList input[type=text]')[$i]; e.focus(); e.select();")
  [void](Send 'Input.insertText' @{ text = $text })
  Start-Sleep -Milliseconds 700
}
function Value([string]$id) { Eval "document.getElementById('$id').value" }
function Text([string]$id) { Eval "document.getElementById('$id').textContent" }
function Shot([string]$n) { $b = (Send 'Page.captureScreenshot' @{ format = 'png' }).data; [IO.File]::WriteAllBytes((Join-Path $OutDir "$n.png"), [Convert]::FromBase64String($b)) }
function Models { Get-Content -LiteralPath $mj -Raw -Encoding UTF8 | ConvertFrom-Json }
function ModelList { @((Models).modelos) }
function Open-App {
  # --hidden: fica na bandeja e nao pega o foco de quem estiver usando o PC.
  $script:proc = Start-Process $exe -ArgumentList '--hidden' -PassThru
  $tg = $null
  foreach ($i in 1..40) { Start-Sleep -Milliseconds 250; try { $tg = @(Invoke-RestMethod 'http://127.0.0.1:9333/json' -TimeoutSec 2 | Where-Object { $_.type -eq 'page' })[0]; if ($tg) { break } } catch {} }
  if (-not $tg) { throw 'Depurador do WebView nao respondeu.' }
  $script:ws = New-Object Net.WebSockets.ClientWebSocket
  $script:ws.ConnectAsync([Uri]$tg.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
  foreach ($i in 1..40) { if ((Value 'model') -or (Eval 'document.getElementById("status").className.indexOf("is-error")>=0')) { break }; Start-Sleep -Milliseconds 150 }
  Start-Sleep -Milliseconds 500
  $script:lastName = $null; Trace 'abrir'
}
function Close-App {
  if ($script:ws) { $script:ws.Dispose(); $script:ws = $null }
  # So o processo que este teste iniciou.
  if ($script:proc -and -not $script:proc.HasExited) { Stop-Process -Id $script:proc.Id -Force; [void]$script:proc.WaitForExit(5000) }
  Start-Sleep -Milliseconds 800
}

New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
[IO.File]::WriteAllText($sj, ('{"idioma":"pt","destino":' + (ConvertTo-Json $dest) + '}'), (New-Object Text.UTF8Encoding($false)))
try {
  Open-App
  $panelHwnd = PanelWindow; $panelVisible = [WinCheck]::IsWindowVisible($panelHwnd); $panelFocused = [WinCheck]::GetForegroundWindow() -eq $panelHwnd
  Check "iniciado com --hidden nao mostra o painel nem pega o foco (janela=$panelHwnd visivel=$panelVisible foco=$panelFocused)" (($panelHwnd -ne [IntPtr]::Zero) -and (-not $panelVisible) -and (-not $panelFocused))
  Check 'nome da pasta vem vazio, pedindo para digitar' ((Value 'name') -eq '' -and (Eval 'document.getElementById("name").placeholder') -eq 'Nome da pasta do projeto')
  Check 'Criar pastas e o modo inicial' ((Eval 'document.getElementById("modeCreate").getAttribute("aria-checked")') -eq 'true')
  Click 'go'
  Check 'sem nome o projeto nao e criado' ((Text 'status') -match 'nome ao projeto' -and @(Get-ChildItem -LiteralPath $dest -Force).Count -eq 0)
  Check 'o cursor vai para o campo de nome' ((Eval 'document.activeElement.id') -eq 'name')
  TypeIn 'name' '   '
  Click 'go'
  Check 'nome so com espacos tambem nao cria' (@(Get-ChildItem -LiteralPath $dest -Force).Count -eq 0)
  [void](Eval "var e=document.getElementById('name'); e.value=''; e.dispatchEvent(new Event('input',{bubbles:true}));"); Start-Sleep -Milliseconds 300
  Check 'previa mostra exemplo sem usar como nome real' ((Text 'tree') -match 'Novo Projeto' -and (Value 'name') -eq '')
  Check 'modelo padrao selecionado' ((Value 'model') -eq 'Comissões')
  Check 'nada gravado antes de editar' (-not (Test-Path -LiteralPath $mj))
  Check 'dois modelos nao existem por acidente' ((Eval 'document.getElementById("model").options.length') -eq 1)

  Click 'tabSettings'
  Shot '1-configuracoes'
  Click 'sOpenAtEnd'
  Check 'alternar abrir pasta salva sem botao' ((Test-Path -LiteralPath $mj) -and (Models).abrirAoFinal -eq $false)
  Check 'arquivo nasce com Comissoes e 7 pastas' (@(ModelList).Count -eq 1 -and @(ModelList)[0].nome -eq 'Comissões' -and @(@(ModelList)[0].pastas).Count -eq 7)
  Check 'Excluir bloqueado com um modelo so' ((Eval 'document.getElementById("sDelete").disabled') -eq $true)

  Click 'sNew'
  Check 'Novo cria modelo' (@(ModelList).Count -eq 2)
  TypeIn 'sName' 'Meu Teste Ação'
  Check 'renomear salva sozinho, com acento' (@(ModelList)[1].nome -eq 'Meu Teste Ação')
  Check 'modelo novo comeca sem pastas' ((FolderCount) -eq 0)
  AddFolder 'A 01'
  Check 'primeira pasta ja nasce como destino' (@(ModelList)[1].pastaDestino -eq 'A 01' -and (FolderRows)[0][1] -eq $true)
  AddFolder 'B 02/Sub'
  # Linha vazia nao vira pasta; Enter cria a proxima logo abaixo
  [void](Eval "document.getElementById('sAddFolder').click()"); Start-Sleep -Milliseconds 500
  Check 'linha ainda sem nome nao e gravada' (@(@(ModelList)[1].pastas).Count -eq 2)
  [void](Send 'Input.insertText' @{ text = '  C 03  ' }); Start-Sleep -Milliseconds 600
  $f = @(@(ModelList)[1].pastas)
  Check "pastas limpas ($($f -join ' | '))" ($f.Count -eq 3 -and $f[1] -eq 'B 02/Sub' -and $f[2] -eq 'C 03')
  [void](Eval 'document.querySelectorAll("#sFolderList input[type=text]")[0].dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true}))'); Start-Sleep -Milliseconds 300
  Check 'Enter cria linha logo abaixo' ((FolderCount) -eq 4 -and (FolderRows)[1][0] -eq '')
  RemoveFolder 1
  Check 'remover a linha vazia nao mexe no destino' ((FolderCount) -eq 3 -and @(ModelList)[1].pastaDestino -eq 'A 01')
  MarkFolder 1
  Check 'marcar outra pasta muda o destino e salva' (@(ModelList)[1].pastaDestino -eq 'B 02/Sub' -and (FolderRows)[1][1] -eq $true -and (FolderRows)[0][1] -eq $false)
  RenameFolder 1 'B 02/Sub2'
  Check 'renomear a marcada leva o destino junto' (@(ModelList)[1].pastaDestino -eq 'B 02/Sub2')
  RenameFolder 1 'B 02/Sub'
  TypeIn 'sRoot' '{nome}'
  Check 'pasta principal salva' (@(ModelList)[1].pastaRaiz -eq '{nome}')
  Check 'previa acompanha' ((Text 'sTree') -match 'A 01')
  Shot '2-editado'

  TypeIn 'sName' 'comissões'
  Check 'nome repetido mostra erro' ((Text 'status') -match 'Já existe')
  Check 'nome repetido nao e gravado' (@(ModelList)[1].nome -eq 'Meu Teste Ação')
  Shot '3-nome-repetido'
  TypeIn 'sName' '   '
  Check 'nome vazio mostra erro' ((Text 'status') -match 'nome ao modelo')
  Check 'nome vazio nao e gravado' (@(ModelList)[1].nome -eq 'Meu Teste Ação')
  TypeIn 'sName' 'Meu Teste Ação'

  Click 'tabProject'
  TypeIn 'name' 'Novo Projeto'
  Check 'modelo novo aparece no Projeto' ((Eval '[...document.getElementById("model").options].map(o=>o.value).join("|")') -eq 'Comissões|Meu Teste Ação')
  Choose 'model' 'Meu Teste Ação'
  Start-Sleep -Milliseconds 300
  Check 'previa do Projeto usa o modelo novo' ((Text 'tree') -match 'Novo Projeto\\\s+A 01')
  Shot '4-projeto'
  # Link colado cai na pasta marcada do modelo; escolha no link vence depois disso.
  [void](Send 'Input.insertText' @{ text = '' })
  [void](Eval '(async()=>{const d=new DataTransfer();d.setData("text","https://drive.google.com/file/d/IdInventadoDoFreyProjectDest00/view");document.getElementById("links").dispatchEvent(new ClipboardEvent("paste",{clipboardData:d,bubbles:true,cancelable:true}));})()'); Start-Sleep -Milliseconds 600
  Check 'link novo entra na pasta marcada' ((Eval 'document.querySelector("#rows select").value') -eq 'B 02\Sub')
  Choose 'model' 'Comissões'
  Check 'trocar de modelo leva o link ao destino do outro' ((Eval 'document.querySelector("#rows select").value') -eq '01 Footage')
  Choose 'model' 'Meu Teste Ação'
  [void](Eval 'var e=document.querySelector("#rows select"); e.value="C 03"; e.dispatchEvent(new Event("change",{bubbles:true}));'); Start-Sleep -Milliseconds 200
  Click 'tabSettings'; MarkFolder 0; Click 'tabProject'; Start-Sleep -Milliseconds 400
  Check 'escolha manual no link nao e trocada' ((Eval 'document.querySelector("#rows select").value') -eq 'C 03')
  [void](Eval 'document.querySelector("#rows .row button").click()'); Start-Sleep -Milliseconds 300
  Click 'go'; Start-Sleep -Milliseconds 700
  Check 'projeto criado com o modelo novo' ((Test-Path -LiteralPath "$dest\Novo Projeto\A 01") -and (Test-Path -LiteralPath "$dest\Novo Projeto\B 02\Sub") -and (Test-Path -LiteralPath "$dest\Novo Projeto\C 03"))
  Check 'nada do modelo antigo' (-not (Test-Path -LiteralPath "$dest\Novo Projeto\01 Footage"))

  # Modelo sem pasta principal nao tem o que nomear: o campo some e cria direto.
  Click 'tabSettings'
  [void](Eval "var e=document.getElementById('sRoot'); e.value=''; e.dispatchEvent(new Event('input',{bubbles:true}));"); Start-Sleep -Milliseconds 700
  Click 'tabProject'; Start-Sleep -Milliseconds 400
  Check 'sem pasta principal o campo de nome some' ((Eval 'document.getElementById("queryRow").hidden') -eq $true)
  [void](Eval "var e=document.getElementById('name'); e.value=''; e.dispatchEvent(new Event('input',{bubbles:true}));"); Start-Sleep -Milliseconds 300
  Click 'go'; Start-Sleep -Milliseconds 500
  Check 'cria direto na pasta escolhida, sem pedir nome' (Test-Path -LiteralPath "$dest\A 01")
  Click 'tabSettings'; TypeIn 'sRoot' '{nome}'; Click 'tabProject'; Start-Sleep -Milliseconds 400
  Check 'com pasta principal o campo volta' ((Eval 'document.getElementById("queryRow").hidden') -eq $false)
  Shot '4b-criar-pastas'

  # So baixar: sem estrutura. O download real so roda com FREY_TEST_DRIVE_FILE: link publico de um arquivo pequeno.
  $before = @(Get-ChildItem -LiteralPath $dest -Directory | ForEach-Object Name) -join '|'
  Click 'modeDownload'; Start-Sleep -Milliseconds 300
  Check 'So baixar esconde nome, modelo e estrutura' ((Eval '["queryRow","fieldModel","tree"].every(i=>document.getElementById(i).hidden)') -eq $true)
  Check 'So baixar deixa o botao Baixar' ((Text 'go') -eq 'Baixar')
  Check 'So baixar troca o rotulo do destino' ((Text 'destLabel') -eq 'Salvar em')
  Check 'So baixar fica lembrado' ((Get-Content -LiteralPath $sj -Raw) -match '"so_baixar":\s*true')
  Click 'go'; Start-Sleep -Milliseconds 300
  Check 'So baixar sem link pede um link' ((Text 'status') -match 'pelo menos um link')
  $driveFile = if ($env:FREY_TEST_DRIVE_FILE) { $env:FREY_TEST_DRIVE_FILE } else { 'https://drive.google.com/file/d/IdInventadoDoFreyProjectTeste000/view' }
  [void](Eval "(async()=>{const d=new DataTransfer();d.setData('text',$(ConvertTo-Json $driveFile));document.getElementById('links').dispatchEvent(new ClipboardEvent('paste',{clipboardData:d,bubbles:true,cancelable:true}));})()"); Start-Sleep -Milliseconds 500
  Check 'linha do link nao mostra pasta do modelo' ((Eval 'getComputedStyle(document.querySelector("#rows select")).display') -eq 'none')
  Shot '4c-so-baixar'
  Click 'go'
  foreach ($i in 1..60) { Start-Sleep -Milliseconds 500; if ((Eval 'document.getElementById("go").disabled') -eq $false) { break } }
  Start-Sleep -Milliseconds 300
  $files = @(Get-ChildItem -LiteralPath $dest -File)
  if ($env:FREY_TEST_DRIVE_FILE) {
    Check "arquivo real caiu direto na pasta ($($files.Count), status: $(Text 'status'))" ($files.Count -eq 1 -and $files[0].Length -gt 1000)
  } else {
    Check 'link invalido falha sem deixar arquivo' ($files.Count -eq 0)
    [void]$results.Add('INFO  download real pulado: defina FREY_TEST_DRIVE_FILE')
  }
  Check 'So baixar nao criou nenhuma pasta' ((@(Get-ChildItem -LiteralPath $dest -Directory | ForEach-Object Name) -join '|') -eq $before)
  Shot '4d-baixado'
  # Sem o arquivo real o link falhou, e clicar de novo seria uma nova tentativa com o app ocupado.
  if ($env:FREY_TEST_DRIVE_FILE) {
    Click 'go'; Start-Sleep -Milliseconds 300
    Check 'clicar de novo sem nada pendente nao baixa duas vezes' (@(Get-ChildItem -LiteralPath $dest -File).Count -eq 1)
  }
  [void](Eval 'document.querySelector("#rows .row button").click()'); Start-Sleep -Milliseconds 300
  Click 'modeCreate'; Start-Sleep -Milliseconds 300
  Check 'voltar a Criar pastas mostra o nome' ((Eval 'document.getElementById("queryRow").hidden') -eq $false -and (Eval 'document.activeElement.id') -eq 'name')

  Click 'tabSettings'
  Click 'sDuplicate'
  Check 'Duplicar copia as pastas' (@(ModelList).Count -eq 3 -and @(@(ModelList)[2].pastas).Count -eq 3 -and @(ModelList)[2].nome -match 'cópia')
  Click 'sDelete'
  Check 'primeiro clique nao exclui' (@(ModelList).Count -eq 3)
  Check 'botao pede confirmacao' ((Text 'sDelete') -match 'de novo')
  Click 'sDelete'
  Check 'segundo clique exclui' (@(ModelList).Count -eq 2)

  # Modelo em uso no Projeto e excluido: o Projeto nao pode ficar apontando para o nada.
  Click 'tabProject'; Choose 'model' 'Meu Teste Ação'; Click 'tabSettings'
  Choose 'sModel' '1'
  Click 'sDelete'; Click 'sDelete'
  Check 'modelo em uso pode ser excluido' (@(ModelList).Count -eq 1)
  Check 'Projeto passa para um modelo que existe' ((Value 'model') -eq 'Comissões')

  # Personalizar o padrao e restaurar
  Check 'Comissoes vem com 01 Footage marcada' (@(ModelList)[0].pastaDestino -eq '01 Footage' -and (FolderRows)[0][1] -eq $true)
  AddFolder '08 Extra'
  Check 'adicionar pasta no Comissoes salva' (@(@(ModelList)[0].pastas).Count -eq 8)
  MarkFolder 7
  Check 'Comissoes aceita outro destino' (@(ModelList)[0].pastaDestino -eq '08 Extra')
  RemoveFolder 7
  Check 'remover a marcada passa o destino para a primeira' (@(ModelList)[0].pastaDestino -eq '01 Footage' -and @(@(ModelList)[0].pastas).Count -eq 7)
  RemoveFolder 6
  Check 'remover outra pasta salva' (@(@(ModelList)[0].pastas).Count -eq 6)
  Click 'sRestore'
  Check 'primeiro clique nao restaura' (@(@(ModelList)[0].pastas).Count -eq 6)
  Click 'sRestore'
  Check 'restaurar volta as 7 pastas' (@(@(ModelList)[0].pastas).Count -eq 7)
  Check 'editor mostra o restaurado' ((FolderCount) -eq 7 -and (FolderRows)[6][0] -eq '07 Legendas' -and (FolderRows)[0][1] -eq $true)

  # Idioma muda tudo na hora e fica salvo
  Choose 'sLanguage' 'ar'
  Check 'arabe inverte o layout' ((Eval 'document.documentElement.dir') -eq 'rtl')
  Check 'textos sem chave faltando' (-not ((Eval 'document.body.innerText') -match '\[\['))
  Shot '5-arabe'
  Choose 'sLanguage' 'en'
  Check 'volta para esquerda-direita' ((Eval 'document.documentElement.dir') -eq 'ltr')
  Check 'aba traduzida' ((Text 'tabSettings') -eq 'Settings')
  Check 'idioma salvo sozinho' ((Get-Content -LiteralPath $sj -Raw) -match '"idioma":\s*"en"')
  # Mensagem ja na tela tambem muda de idioma
  Choose 'sLanguage' 'pt'

  # Nome digitado pelo usuario nao pode ser trocado ao mudar o idioma
  Click 'tabProject'; TypeIn 'name' 'Cliente Real'; Click 'tabSettings'
  Choose 'sLanguage' 'es'
  Check 'nome digitado nao muda com o idioma' ((Value 'name') -eq 'Cliente Real')
  Choose 'sLanguage' 'en'

  # Reiniciar preserva
  TypeIn 'sName' 'Comissões'
  Click 'sNew'; TypeIn 'sName' 'Sobrevive'
  # Esconder devolve o foco: nada de janela invisivel recebendo o teclado.
  [void](Start-Process $exe)   # segunda execucao, sem --hidden: traz o painel
  Start-Sleep -Milliseconds 1200
  $h = PanelWindow
  [void]$results.Add("INFO  segunda execucao: visivel=$([WinCheck]::IsWindowVisible($h)) foco=$([WinCheck]::GetForegroundWindow() -eq $h)")
  Check 'segunda execucao mostra o painel' ([WinCheck]::IsWindowVisible($h))
  Click 'hide'; Start-Sleep -Milliseconds 500
  Check 'botao esconder oculta o painel' (-not [WinCheck]::IsWindowVisible($h))
  Check 'esconder nao deixa o foco na janela invisivel' ([WinCheck]::GetForegroundWindow() -ne $h)
  [void](Start-Process $exe -ArgumentList '--hidden'); Start-Sleep -Milliseconds 900
  Check 'segunda execucao com --hidden nao traz o painel' (-not [WinCheck]::IsWindowVisible($h))
  Close-App
  Open-App
  Check 'idioma continua apos reiniciar' ((Text 'tabSettings') -eq 'Settings')
  Check 'modelo continua apos reiniciar' ((Eval '[...document.getElementById("model").options].map(o=>o.value).join("|")') -match 'Sobrevive')
  Close-App

  # Arquivo corrompido
  Set-Content -LiteralPath $mj -Value '{ quebrado' -Encoding Ascii
  Open-App
  Shot '6-corrompido'
  Check 'arquivo com erro nao e sobrescrito sozinho' ((Get-Content -LiteralPath $mj -Raw).Trim() -eq '{ quebrado')
  Check 'abre em Configuracoes mostrando o erro' ((Eval 'document.getElementById("viewSettings").hidden') -eq $false -and (Text 'status') -match 'error')
  Check 'Projeto fica bloqueado' ((Eval 'document.getElementById("name").disabled') -eq $true)
  Click 'tabConvert'
  Check 'Converter continua livre' ((Eval 'document.getElementById("cAdd").disabled') -eq $false)
  Click 'tabSettings'
  Click 'sRestore'; Click 'sRestore'
  $ok = $false; try { $ok = @(@(ModelList)[0].pastas).Count -eq 7 } catch {}
  Check 'restaurar recupera do arquivo com erro' $ok
  Check 'guarda o arquivo com erro em .bak' ((Get-Content -LiteralPath "$cfgDir\modelos.json.bak" -Raw).Trim() -eq '{ quebrado')
  Check 'Projeto volta a funcionar' ((Eval 'document.getElementById("name").disabled') -eq $false -and (Value 'model') -eq 'Comissões')
  Shot '7-recuperado'
} catch {
  Check "erro: $($_.Exception.Message)" $false
  try { Shot 'erro' } catch {}
} finally {
  Close-App
  Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  Remove-Item Env:FREY_PROJECT_CONFIG_DIR -ErrorAction SilentlyContinue
}
$results | ForEach-Object { $_ }
"Saida: $OutDir"
if (@($results | Where-Object { $_ -like 'FAIL*' }).Count) { exit 1 }
