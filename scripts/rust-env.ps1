# Configure l'environnement Rust GNU (rust-mingw + w64devkit + WebView2Loader) et exécute la commande passée.
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Command
)

$ErrorActionPreference = "Stop"
$rustBin = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin"
$selfContained = "$rustBin\self-contained"
$w64Bin = "$env:USERPROFILE\.local\w64devkit\bin"

if (Test-Path "$selfContained\x86_64-w64-mingw32-gcc.exe") {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "$selfContained\x86_64-w64-mingw32-gcc.exe"
}

$extraPaths = @("$env:USERPROFILE\.cargo\bin", $rustBin, $w64Bin)
$wv2 = Get-ChildItem "$PSScriptRoot\..\src-tauri\target" -Recurse -Filter "WebView2Loader.dll" -ErrorAction SilentlyContinue | Where-Object { $_.FullName -like "*\x64\WebView2Loader.dll" } | Select-Object -First 1
if ($wv2) {
    $extraPaths += $wv2.DirectoryName
}

$env:PATH = (($extraPaths | Where-Object { Test-Path $_ }) -join ";") + ";" + $env:PATH

if ($Command.Count -gt 0) {
    $exe = $Command[0]
    $rest = if ($Command.Count -gt 1) { $Command[1..($Command.Count - 1)] } else { @() }
    & $exe @rest
    exit $LASTEXITCODE
}
