# Build evdict.exe va file du lieu evdict.dat vao thu muc dist\
$ErrorActionPreference = "Stop"
$env:Path += ";$env:USERPROFILE\.cargo\bin"
Set-Location $PSScriptRoot

& "$PSScriptRoot\scripts\fetch-data.ps1"

cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }

New-Item -ItemType Directory -Force dist | Out-Null
$name = "Anh - Vi" + [char]0x1EC7 + "t"   # "Anh - Việt"; PowerShell 5.1 đọc file .ps1 theo bảng mã ANSI
& .\target\release\builddict.exe data\raw\x\stardict_en_vi\en_vi dist\evdict.dat $name
if ($LASTEXITCODE -ne 0) { exit 1 }
Copy-Item target\release\evdict.exe dist\evdict.exe -Force

Get-ChildItem dist | Select-Object Name, @{n="Size (KB)"; e={[math]::Round($_.Length / 1KB)}}
