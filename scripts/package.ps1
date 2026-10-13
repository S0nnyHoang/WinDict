# Dong goi tu dist\ (chay build.ps1 truoc) ra thu muc out\:
#   EVDict-Portable-<version>.zip  giai nen la chay, du lieu luu canh exe (file portable.txt)
#   EVDict-Setup-<version>.exe     trinh cai dat Inno Setup (neu co ISCC.exe)
param(
    [string]$Version,
    [switch]$RequireInstaller
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not $Version) {
    $m = Select-String -Path evdict\Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    $Version = $m.Matches[0].Groups[1].Value
}
foreach ($f in "dist\evdict.exe", "dist\evdict.dat") {
    if (-not (Test-Path $f)) { throw "$f not found - run build.ps1 first" }
}
New-Item -ItemType Directory -Force out | Out-Null

# --- Ban portable ---
$stage = Join-Path $root "out\EVDict"
if (Test-Path $stage) { [IO.Directory]::Delete($stage, $true) }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item dist\evdict.exe, dist\evdict.dat, README.md $stage
@"
EVDict portable

File nay danh dau ban portable: tu dien them vao duoc luu trong thu muc data\ ngay ben canh,
khong ghi vao %APPDATA%. Xoa file nay neu muon dung nhu ban cai dat binh thuong.
(Phim tat va tuy chon "Khoi dong cung Windows" van luu trong registry cua nguoi dung.)
"@ | Set-Content -Encoding ascii (Join-Path $stage "portable.txt")
$portable = Join-Path $root "out\EVDict-Portable-$Version.zip"
if (Test-Path $portable) { [IO.File]::Delete($portable) }
Compress-Archive -Path $stage -DestinationPath $portable -CompressionLevel Optimal
Write-Host "Portable: $portable"

# --- Trinh cai dat ---
$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($iscc) {
    & $iscc /Q "/DAppVersion=$Version" "/O$root\out" installer\evdict.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
    Write-Host "Installer: $root\out\EVDict-Setup-$Version.exe"
} elseif ($RequireInstaller) {
    throw "Inno Setup (ISCC.exe) not found"
} else {
    Write-Warning "Inno Setup not found - skipped installer (portable zip was built)"
}
