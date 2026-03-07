$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$VenvDir = Join-Path $ScriptDir "venv"
$DataDir = Join-Path $ScriptDir "data"

Write-Host "==============================="
Write-Host "  GeoRAG - Geotechnical RAG"
Write-Host "==============================="
Write-Host ""

# Check Python 3.11
$Python = $null
if (Get-Command python3.11 -ErrorAction SilentlyContinue) {
    $Python = "python3.11"
} elseif (Get-Command python -ErrorAction SilentlyContinue) {
    $PyVersion = & python --version 2>&1
    Write-Host "Using $PyVersion (3.11 recommended)"
    $Python = "python"
} elseif (Get-Command python3 -ErrorAction SilentlyContinue) {
    $PyVersion = & python3 --version 2>&1
    Write-Host "Using $PyVersion (3.11 recommended)"
    $Python = "python3"
} else {
    Write-Host "ERROR: Python not found. Please install Python 3.11+"
    exit 1
}

# Create venv if needed
if (-not (Test-Path $VenvDir)) {
    Write-Host "Creating virtual environment..."
    & $Python -m venv $VenvDir
    Write-Host "Installing dependencies..."
    & "$VenvDir\Scripts\pip.exe" install --upgrade pip -q
    & "$VenvDir\Scripts\pip.exe" install -r (Join-Path $ScriptDir "requirements.txt") -q
    Write-Host "Dependencies installed."
} else {
    Write-Host "Virtual environment found."
}

# Create data directories
foreach ($sub in @("chroma", "cache", "logs")) {
    $dir = Join-Path $DataDir $sub
    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
}

# Check LM Studio
Write-Host ""
try {
    $null = Invoke-WebRequest -Uri "http://127.0.0.1:1234/v1/models" -TimeoutSec 2 -ErrorAction Stop
    Write-Host "LM Studio: Connected"
} catch {
    Write-Host "WARNING: LM Studio not detected at http://127.0.0.1:1234"
    Write-Host "  Chat and embedding features will not work until LM Studio is running."
}

Write-Host ""
Write-Host "Starting GeoRAG server..."
Write-Host "  URL: http://localhost:3000"
Write-Host ""

Set-Location $ScriptDir
& "$VenvDir\Scripts\uvicorn.exe" backend.app:app --host 0.0.0.0 --port 3000 --reload
