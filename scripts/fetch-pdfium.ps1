# Télécharge PDFium (moteur PDF de Chromium, licence BSD-3/Apache-2.0)
# depuis les binaires officiels de bblanchon/pdfium-binaries, vérifie
# l'empreinte SHA-256 puis place pdfium.dll dans src-tauri/resources/pdfium.
#
# La version est figée sur celle que supporte pdfium-render (feature pdfium_8125).
# Pour mettre à jour : changer $Tag ET $Sha256 ensemble, puis relancer les tests.

$ErrorActionPreference = 'Stop'

$Tag    = 'chromium/8125'
$Asset  = 'pdfium-win-x64.tgz'
$Sha256 = '9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b'

$Root   = Split-Path -Parent $PSScriptRoot
$Dest   = Join-Path $Root 'src-tauri\resources\pdfium'
$Dll    = Join-Path $Dest 'pdfium.dll'

if (Test-Path $Dll) {
    Write-Host "pdfium.dll deja present : $Dll"
    exit 0
}

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("hestia-pdfium-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $Work | Out-Null
try {
    $Archive = Join-Path $Work $Asset
    $Url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$Tag/$Asset"
    Write-Host "Telechargement de $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Archive -UseBasicParsing

    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if ($Actual -ne $Sha256) {
        throw "Empreinte SHA-256 invalide pour $Asset : attendu $Sha256, obtenu $Actual"
    }
    Write-Host 'Empreinte SHA-256 verifiee.'

    $Extract = Join-Path $Work 'x'
    New-Item -ItemType Directory -Force $Extract | Out-Null
    tar -xzf $Archive -C $Extract
    if ($LASTEXITCODE -ne 0) { throw "Echec de l'extraction de $Asset" }

    New-Item -ItemType Directory -Force $Dest | Out-Null
    Copy-Item (Join-Path $Extract 'bin\pdfium.dll') $Dll
    Copy-Item (Join-Path $Extract 'LICENSE') (Join-Path $Dest 'LICENSE-pdfium.txt')
    if (Test-Path (Join-Path $Extract 'licenses')) {
        Copy-Item -Recurse -Force (Join-Path $Extract 'licenses') (Join-Path $Dest 'licenses')
    }
    Write-Host "PDFium installe dans $Dest"
}
finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
