# Tai du lieu tu dien Anh-Viet (OVDP, GPLv2) va giai nen vao data\raw\x.
# Dung cho ca may dev lan CI. Co kiem tra SHA-256; da co file dung thi bo qua.
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$zip = Join-Path $root "data\raw\AnhViet.zip"
$out = Join-Path $root "data\raw\x"
$sha = "6FF1508004311E916993B4EB84580CEE910C89D682028D387763F7F85FC9C442"
$url = "https://sourceforge.net/projects/ovdp/files/Stardict/English/AnhViet.zip/download"

function Test-Zip { (Test-Path $zip) -and (Get-FileHash $zip -Algorithm SHA256).Hash -eq $sha }

New-Item -ItemType Directory -Force (Split-Path $zip) | Out-Null
if (-not (Test-Zip)) {
    for ($i = 1; $i -le 3 -and -not (Test-Zip); $i++) {
        Write-Host "Downloading AnhViet.zip (attempt $i)..."
        curl.exe -L --fail --silent --show-error --retry 3 -o $zip $url
        if (-not (Test-Zip)) { Start-Sleep -Seconds 5 }
    }
    if (-not (Test-Zip)) { throw "AnhViet.zip: download failed or SHA-256 mismatch" }
}
if (-not (Test-Path (Join-Path $out "stardict_en_vi\en_vi.ifo"))) {
    Expand-Archive $zip -DestinationPath $out -Force
}
Write-Host "Dictionary data ready: $out"
