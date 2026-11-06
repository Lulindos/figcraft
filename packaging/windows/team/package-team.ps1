<#
.SYNOPSIS
  Monta o zip do LegatIA Design para o time (Windows x64): app + CLI + sincronizador + instalador.

.DESCRIPTION
  Produz em $env:DIST (padrão dist/release):
    figcraft-<versão>-windows-x64-portable.zip   (o nome que o verificador de atualização do app procura)
    SHA256SUMS.txt

  Dentro do zip, numa pasta só: figcraft.exe, figcraft-cli.exe, legatia-sync\{node.exe,sync.mjs},
  install.ps1, Instalar.cmd, LEIA-ME.txt, VERSION e as licenças. Não leva o arquivo "portable":
  as preferências ficam em %APPDATA%, e o app se atualiza por cima da pasta de instalação.

  Precisa de: o build já feito (cargo build --release --target x86_64-pc-windows-msvc com
  CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'), o node.exe e o
  sync.mjs empacotado (no repositório LegatIAdm: npm run design:sync:bundle).

.EXAMPLE
  pwsh packaging/windows/team/package-team.ps1 -SyncDir C:\...\dist\legatia-sync
#>
param(
  [Parameter(Mandatory)] [string] $SyncDir,
  [string] $NodeExe = (Get-Command node).Source
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$Target = 'x86_64-pc-windows-msvc'
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$Bin = Join-Path $TargetDir "$Target\release"

$Version = $null
$inPkg = $false
foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
  if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
  if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
}
if (-not $Version) { throw 'não achei [workspace.package] version em Cargo.toml' }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "a versão $Version não é estável (X.Y.Z): o verificador de atualização só oferece versões estáveis" }

foreach ($f in (Join-Path $Bin 'figcraft.exe'), (Join-Path $Bin 'figcraft-cli.exe'), $NodeExe, (Join-Path $SyncDir 'sync.mjs')) {
  if (-not (Test-Path -LiteralPath $f)) { throw "faltou: $f" }
}

$Name = "figcraft-$Version-windows-x64-portable"
$Stage = Join-Path $TargetDir "windows-package\$Name"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'legatia-sync') | Out-Null
Copy-Item (Join-Path $Bin 'figcraft.exe'), (Join-Path $Bin 'figcraft-cli.exe') $Stage
Copy-Item $NodeExe (Join-Path $Stage 'legatia-sync\node.exe')
Copy-Item (Join-Path $SyncDir 'sync.mjs') (Join-Path $Stage 'legatia-sync\sync.mjs')
foreach ($f in 'install.ps1', 'Instalar.cmd', 'LEIA-ME.txt') { Copy-Item (Join-Path $PSScriptRoot $f) $Stage }
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Stage }
}
Set-Content -LiteralPath (Join-Path $Stage 'VERSION') -Value $Version -NoNewline -Encoding ascii

# O zip tem uma pasta só, com barras "/" nos nomes (o Compress-Archive do PowerShell 5 grava "\").
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Zip = Join-Path $Dist "$Name.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::Open($Zip, 'Create')
try {
  $base = (Resolve-Path $Stage).Path.TrimEnd('\') + '\'
  foreach ($file in Get-ChildItem -LiteralPath $Stage -Recurse -File) {
    $entry = $Name + '/' + $file.FullName.Substring($base.Length).Replace('\', '/')
    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName, $entry, [System.IO.Compression.CompressionLevel]::Optimal)
  }
} finally { $archive.Dispose() }

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Zip).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $Dist 'SHA256SUMS.txt') -Value "$hash  $Name.zip" -Encoding ascii
Get-Item $Zip | Format-Table Name, Length
Write-Output "sha256 $hash"
