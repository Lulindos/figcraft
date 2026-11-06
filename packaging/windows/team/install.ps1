<#
.SYNOPSIS
  Instala o LegatIA Design (app nativo) para a pessoa que está logada, sem precisar de administrador.

.DESCRIPTION
  Copia esta pasta para %LOCALAPPDATA%\Programs\LegatIA Design, cria os atalhos (menu Iniciar e
  área de trabalho), associa os arquivos .figcraft ao app e registra a desinstalação em
  Configurações > Aplicativos. Rodar de novo atualiza a instalação. Depois da primeira vez o próprio
  app avisa das versões novas e se atualiza.

  Os seus arquivos (a pasta "LegatIA Design" no seu usuário e as preferências do app) nunca são tocados.

.EXAMPLE
  .\install.ps1                      instala ou atualiza
  .\install.ps1 -SemAtalhoNaArea     sem atalho na área de trabalho
  .\install.ps1 -Desinstalar         remove o app (os seus arquivos ficam)
#>
param(
  [string] $Destino = (Join-Path $env:LOCALAPPDATA 'Programs\LegatIA Design'),
  [switch] $SemAtalhoNaArea,
  [switch] $Desinstalar
)
$ErrorActionPreference = 'Stop'
$Nome = 'LegatIA Design'
$Origem = $PSScriptRoot
$Exe = Join-Path $Destino 'figcraft.exe'
$Menu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$Nome.lnk"
$Area = Join-Path ([Environment]::GetFolderPath('Desktop')) "$Nome.lnk"
$ChaveApp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LegatIADesign'
$ClasseArq = 'HKCU:\Software\Classes\LegatIADesign.Arquivo'

function Close-Running {
  # O app e o sincronizador que ele abre (node.exe dentro da pasta de instalação) seguram os arquivos.
  $prefixo = ([IO.Path]::GetFullPath($Destino)).TrimEnd('\') + '\'
  $abertos = Get-Process -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and $_.Path.StartsWith($prefixo, [StringComparison]::OrdinalIgnoreCase)
  }
  if (-not $abertos) { return }
  Write-Output "O $Nome está aberto. Feche o app (salve o que estiver aberto) e rode de novo."
  Write-Output ('Abertos: ' + (($abertos | ForEach-Object { $_.ProcessName } | Sort-Object -Unique) -join ', '))
  throw 'Instalação interrompida: nada foi alterado.'
}

function Remove-Registrations {
  Remove-Item -LiteralPath $Menu, $Area -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $ChaveApp -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $ClasseArq -Recurse -Force -ErrorAction SilentlyContinue
  # Só solta a associação se ainda for a nossa.
  $ext = 'HKCU:\Software\Classes\.figcraft'
  if ((Get-ItemProperty -LiteralPath $ext -ErrorAction SilentlyContinue).'(default)' -eq 'LegatIADesign.Arquivo') {
    Remove-Item -LiteralPath $ext -Recurse -Force -ErrorAction SilentlyContinue
  }
}

if ($Desinstalar) {
  Close-Running
  Remove-Registrations
  # Esta cópia do script pode estar dentro da pasta que vai embora: a limpeza roda depois que o PowerShell sai.
  if (Test-Path -LiteralPath $Destino) {
    Start-Process cmd.exe -WindowStyle Hidden -ArgumentList '/c', "ping -n 3 127.0.0.1 >nul & rmdir /s /q `"$Destino`""
  }
  Write-Output "$Nome removido. A sua pasta de arquivos e as preferências do app continuam onde estavam."
  return
}

if (-not (Test-Path -LiteralPath (Join-Path $Origem 'figcraft.exe'))) {
  throw "Não achei figcraft.exe ao lado deste script ($Origem). Extraia o zip inteiro antes de instalar."
}

Close-Running
New-Item -ItemType Directory -Force -Path $Destino | Out-Null
$mesma = [IO.Path]::GetFullPath($Origem).TrimEnd('\') -eq [IO.Path]::GetFullPath($Destino).TrimEnd('\')
if (-not $mesma) {
  Write-Output "Copiando para $Destino ..."
  Copy-Item -Path (Join-Path $Origem '*') -Destination $Destino -Recurse -Force
}
# Arquivos baixados do navegador vêm marcados; sem isso o Windows pergunta de novo a cada abertura do app.
Get-ChildItem -LiteralPath $Destino -Recurse -File | Unblock-File -ErrorAction SilentlyContinue

# Atalhos
$shell = New-Object -ComObject WScript.Shell
$atalhos = @($Menu)
if (-not $SemAtalhoNaArea) { $atalhos += $Area }
foreach ($caminho in $atalhos) {
  $s = $shell.CreateShortcut($caminho)
  $s.TargetPath = $Exe
  $s.WorkingDirectory = $Destino
  $s.IconLocation = "$Exe,0"
  $s.Description = 'Editor de design da Legat.IA'
  $s.Save()
}

# Arquivos .figcraft abrem no app (só para esta pessoa, sem administrador).
New-Item -Path $ClasseArq -Force | Out-Null
Set-ItemProperty -Path $ClasseArq -Name '(default)' -Value 'Arquivo do LegatIA Design'
New-Item -Path "$ClasseArq\DefaultIcon" -Force | Out-Null
Set-ItemProperty -Path "$ClasseArq\DefaultIcon" -Name '(default)' -Value "`"$Exe`",0"
New-Item -Path "$ClasseArq\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "$ClasseArq\shell\open\command" -Name '(default)' -Value "`"$Exe`" `"%1`""
New-Item -Path 'HKCU:\Software\Classes\.figcraft' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\.figcraft' -Name '(default)' -Value 'LegatIADesign.Arquivo'

# Configurações > Aplicativos
$versaoArq = Join-Path $Destino 'VERSION'
$versao = if (Test-Path -LiteralPath $versaoArq) { (Get-Content -LiteralPath $versaoArq -Raw).Trim() } else { '' }
New-Item -Path $ChaveApp -Force | Out-Null
$props = @{
  DisplayName = $Nome; Publisher = 'Legat.IA'; DisplayIcon = "$Exe,0"; InstallLocation = $Destino; DisplayVersion = $versao
  UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $Destino 'install.ps1')`" -Desinstalar"
}
foreach ($k in $props.Keys) { Set-ItemProperty -Path $ChaveApp -Name $k -Value $props[$k] }
Set-ItemProperty -Path $ChaveApp -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $ChaveApp -Name NoRepair -Value 1 -Type DWord

# A pasta que o app e o site usam para os arquivos.
New-Item -ItemType Directory -Force -Path (Join-Path $env:USERPROFILE 'LegatIA Design') | Out-Null

Write-Output ''
Write-Output "$Nome $versao instalado em $Destino"
Write-Output 'Abra pelo menu Iniciar. Depois, no site, em Design > "App no computador", clique em "Conectar à minha conta".'
