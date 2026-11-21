# Installation des prérequis Rust + MSVC Build Tools + Tauri v2 sur Windows
# Exécuter dans un terminal PowerShell (sans droits admin requis pour rustup).

$ErrorActionPreference = "Stop"

Write-Host "=== 1. Vérification de rustup / cargo ===" -ForegroundColor Cyan
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Host "Téléchargement de rustup-init.exe..."
    $rustupPath = Join-Path $env:TEMP "rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupPath
    & $rustupPath -y --default-toolchain stable
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
} else {
    Write-Host "rustup déjà installé : $(rustup --version)"
}

Write-Host "=== 2. Vérification des Visual Studio C++ Build Tools (MSVC) ===" -ForegroundColor Cyan
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
    Write-Warning "MSVC Build Tools non détecté. Installation recommandée via winget :"
    Write-Host "  winget install Microsoft.VisualStudio.2022.BuildTools --override `"--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`""
} else {
    Write-Host "Visual Studio détecté : $(& $vswhere -latest -property displayName)"
}

Write-Host "=== 3. Installation des dépendances frontend (@tauri-apps/cli) ===" -ForegroundColor Cyan
npm install

Write-Host "=== 4. Vérification compilation & tests Rust ===" -ForegroundColor Cyan
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml

Write-Host "=== Prêt ! Lancez le launcher avec : npm run dev ===" -ForegroundColor Green
