<#
.SYNOPSIS
  Liga as IAs deste computador ao LegatIA Design (MCP), para elas mexerem no editor.

.DESCRIPTION
  O app abre com um canal local (127.0.0.1, porta 47800, só este computador). O programa figcraft-cli
  faz a ponte entre esse canal e a IA, no padrão MCP. Este script registra a ponte nas IAs que achar:
    - Claude Desktop      (claude_desktop_config.json)
    - Claude Code         (claude mcp add, para o seu usuário)
    - Cursor              (~\.cursor\mcp.json)
  e mostra o trecho pronto para qualquer outra (Windsurf, Cline, Codex...), que também fica copiado.

  Duas entradas são criadas:
    legatia-design           mexe no app aberto, ao vivo (o app precisa estar aberto)
    legatia-design-offline   edita um arquivo sem abrir o app (sem janela; para tarefas em lote)

  Antes de mudar o arquivo de uma IA, guarda uma cópia ao lado (.bak-legatia). Nada é enviado para fora.

.EXAMPLE
  .\connect-ai.ps1            pergunta uma a uma
  .\connect-ai.ps1 -Sim       conecta todas as que achar, sem perguntar
  .\connect-ai.ps1 -Remover   tira as entradas da LegatIA de todas
#>
param(
  [string] $Destino = $PSScriptRoot,
  [int] $Porta = 47800,
  [switch] $Sim,
  [switch] $Remover,
  # Caminhos para teste (ou instalações fora do padrão)
  [string] $ClaudeDesktopConfig,
  [string] $CursorConfig,
  [switch] $SemClaudeCode
)
$ErrorActionPreference = 'Stop'
$Cli = Join-Path $Destino 'figcraft-cli.exe'
if (-not (Test-Path -LiteralPath $Cli)) { throw "Não achei figcraft-cli.exe em $Destino" }
$Nomes = @('legatia-design', 'legatia-design-offline')

function Get-Entries {
  [ordered]@{
    'legatia-design'         = [ordered]@{ command = $Cli; args = @('mcp', '--attach', "$Porta") }
    'legatia-design-offline' = [ordered]@{ command = $Cli; args = @('mcp') }
  }
}

function Confirm-Step([string] $pergunta) {
  if ($Sim) { return $true }
  $r = Read-Host "$pergunta [S/n]"
  return ($r -eq '' -or $r -match '^[sSyY]')
}

# Soma (ou tira) as entradas do bloco "mcpServers" de um arquivo JSON de configuração.
function Set-McpFile([string] $caminho, [bool] $remover) {
  $existe = Test-Path -LiteralPath $caminho
  $obj = $null
  if ($existe) {
    $texto = Get-Content -LiteralPath $caminho -Raw -Encoding UTF8
    if ($texto.Trim()) { $obj = $texto | ConvertFrom-Json }
    Copy-Item -LiteralPath $caminho -Destination "$caminho.bak-legatia" -Force
  }
  if (-not $obj) { $obj = [pscustomobject]@{} }
  if (-not $obj.PSObject.Properties['mcpServers']) { $obj | Add-Member -NotePropertyName mcpServers -NotePropertyValue ([pscustomobject]@{}) }
  foreach ($nome in $Nomes) { $obj.mcpServers.PSObject.Properties.Remove($nome) }
  if (-not $remover) {
    foreach ($e in (Get-Entries).GetEnumerator()) {
      $obj.mcpServers | Add-Member -NotePropertyName $e.Key -NotePropertyValue ([pscustomobject]$e.Value)
    }
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $caminho) | Out-Null
  $json = $obj | ConvertTo-Json -Depth 20
  # UTF-8 sem BOM, como as IAs gravam
  [IO.File]::WriteAllText($caminho, $json, (New-Object Text.UTF8Encoding($false)))
}

$feitos = @()

# ---- Claude Desktop ----------------------------------------------------------------------------
$desktop = $ClaudeDesktopConfig
if (-not $desktop) {
  $candidatos = @(Join-Path $env:APPDATA 'Claude\claude_desktop_config.json')
  $candidatos += @(Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Packages') -Directory -Filter 'Claude_*' -ErrorAction SilentlyContinue |
    ForEach-Object { Join-Path $_.FullName 'LocalCache\Roaming\Claude\claude_desktop_config.json' })
  $desktop = $candidatos | Where-Object { Test-Path -LiteralPath (Split-Path -Parent $_) } | Select-Object -First 1
}
if ($desktop) {
  if (Confirm-Step "Claude Desktop encontrado ($desktop). $(if ($Remover) { 'Remover' } else { 'Conectar' })?") {
    Set-McpFile $desktop $Remover.IsPresent
    $feitos += 'Claude Desktop (feche e abra o Claude Desktop de novo)'
  }
}

# ---- Cursor -------------------------------------------------------------------------------------
$cursor = $CursorConfig
if (-not $cursor -and (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.cursor'))) { $cursor = Join-Path $env:USERPROFILE '.cursor\mcp.json' }
if ($cursor) {
  if (Confirm-Step "Cursor encontrado ($cursor). $(if ($Remover) { 'Remover' } else { 'Conectar' })?") {
    Set-McpFile $cursor $Remover.IsPresent
    $feitos += 'Cursor (reinicie o Cursor)'
  }
}

# ---- Claude Code --------------------------------------------------------------------------------
$claude = if ($SemClaudeCode) { $null } else { Get-Command claude -ErrorAction SilentlyContinue }
if ($claude) {
  if (Confirm-Step "Claude Code encontrado. $(if ($Remover) { 'Remover' } else { 'Conectar' }) para o seu usuário?") {
    foreach ($nome in $Nomes) { & $claude.Source mcp remove $nome --scope user 2>$null | Out-Null }
    if (-not $Remover) {
      & $claude.Source mcp add --scope user legatia-design -- $Cli mcp --attach "$Porta" | Out-Null
      & $claude.Source mcp add --scope user legatia-design-offline -- $Cli mcp | Out-Null
    }
    $feitos += 'Claude Code (vale nas próximas conversas)'
  }
}

# ---- Qualquer outra IA --------------------------------------------------------------------------
$trecho = [pscustomobject]@{ mcpServers = [pscustomobject](Get-Entries) } | ConvertTo-Json -Depth 20
Write-Output ''
if ($feitos.Count) { Write-Output 'Conectado em:'; $feitos | ForEach-Object { Write-Output "  - $_" } }
else { Write-Output 'Nenhuma IA conectada.' }
if (-not $Remover) {
  Write-Output ''
  Write-Output 'Para outra IA (Windsurf, Cline, Codex...), cole este trecho no arquivo de MCP dela:'
  Write-Output $trecho
  try { Set-Clipboard -Value $trecho; Write-Output '(o trecho já está copiado)' } catch { }
  Write-Output ''
  Write-Output "Para usar ao vivo, deixe o LegatIA Design aberto (ele abre o canal local na porta $Porta)."
}
