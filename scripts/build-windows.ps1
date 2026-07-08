<#
.SYNOPSIS
  Tempo — build the Windows app by driving the MSYS2 UCRT64 shell from Windows.

.DESCRIPTION
  A thin wrapper so you don't have to open the MSYS2 shell yourself. It finds
  MSYS2 (C:\msys64, or $env:MSYS2_ROOT) and runs scripts/build-windows.sh inside
  the UCRT64 environment (which is where gfortran/cmake/FFTW live). All extra
  arguments are forwarded to the bash script.

.EXAMPLE
  # From PowerShell, in the repo root:
  .\scripts\build-windows.ps1                 # release build, with live radio
  .\scripts\build-windows.ps1 --no-radio       # UI only
  .\scripts\build-windows.ps1 --check          # verify the toolchain only

  # If script execution is blocked:
  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
#>
$ErrorActionPreference = 'Stop'

$msys = if ($env:MSYS2_ROOT) { $env:MSYS2_ROOT } else { 'C:\msys64' }
$bash = Join-Path $msys 'usr\bin\bash.exe'
if (-not (Test-Path $bash)) {
  Write-Host "MSYS2 not found at '$msys'." -ForegroundColor Red
  Write-Host "Install it from https://www.msys2.org (then use it once to update), or" -ForegroundColor Yellow
  Write-Host "set `$env:MSYS2_ROOT to your MSYS2 install path and re-run." -ForegroundColor Yellow
  exit 1
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path     # ...\tempo\scripts
$hereFwd = $here -replace '\\', '/'

# Run inside the UCRT64 login shell so the MinGW toolchain is on PATH.
$env:MSYSTEM = 'UCRT64'
$env:CHERE_INVOKING = '1'

$u = (& $bash -lc "cygpath -u '$hereFwd'").Trim()
$forward = ($args -join ' ')
Write-Host "Running build-windows.sh in MSYS2 UCRT64 ($msys)…" -ForegroundColor Cyan
& $bash -lc "'$u/build-windows.sh' $forward"
exit $LASTEXITCODE
