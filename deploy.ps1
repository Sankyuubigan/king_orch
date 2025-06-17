# ===================================================================================
# UNIVERSAL SCRIPT FOR AUTONOMOUS DEPLOYMENT OF AI SYSTEM VIA PODMAN
# VERSION 6.0 (FINAL, RELIABLE, ASCII-ONLY, CORRECT COMMANDS AND CONFIGS)
# ===================================================================================

# --- SETTINGS ---
$ScriptRoot = $PSScriptRoot
$ProjectDir = Join-Path $ScriptRoot "Orchestrator"
$ArchiveDir = Join-Path $ScriptRoot "Image_Archives"

$OllamaImage = "docker.io/ollama/ollama:latest"
$OllamaArchiveFile = "ollama.tar"

# Using the new, correct image name for AnythingLLM
$AppImage = "docker.io/mintplexlabs/anythingllm:latest"
$AppArchiveFile = "anything-llm.tar"

# --- SCRIPT START ---

function Write-Log {
    param(
        [string]$Message,
        [string]$Color
    )
    Write-Host $Message -ForegroundColor $Color
}

# --- STEP 1/5: PREPARE ENVIRONMENT ---

Write-Log "--- [STEP 1/5] Preparing environment and configuration... ---" -Color Green

New-Item -Path $ProjectDir -ItemType Directory -Force | Out-Null
New-Item -Path $ArchiveDir -ItemType Directory -Force | Out-Null

# Creating docker-compose.yml with the corrected GPU configuration
$ComposeLines = @(
    "services:",
    "  ollama:",
    "    image: $OllamaImage",
    "    container_name: ollama",
    "    environment:",
    "      - NVIDIA_VISIBLE_DEVICES=all",
    "    volumes:",
    "      - ollama_data:/root/.ollama",
    "    ports:",
    "      - '11434:11434'",
    "    networks:",
    "      - ai_net",
    "    restart: always",
    "",
    "  anything-llm:",
    "    image: $AppImage",
    "    container_name: anything-llm",
    "    depends_on:",
    "      - ollama",
    "    ports:",
    "      - '3001:3001'",
    "    environment:",
    "      - STORAGE_DIR=/app/server/storage",
    "    volumes:",
    "      - anything_storage:/app/server/storage",
    "    networks:",
    "      - ai_net",
    "    restart: always",
    "",
    "networks:",
    "  ai_net:",
    "    driver: bridge",
    "",
    "volumes:",
    "  ollama_data:",
    "  anything_storage:"
)

$ComposeLines | Set-Content -Path (Join-Path $ProjectDir "docker-compose.yml")
Write-Log "docker-compose.yml created successfully with correct GPU settings." -Color Cyan

# --- STEP 2/5: INITIALIZE AND START PODMAN MACHINE ---

Write-Log "--- [STEP 2/5] Initializing and starting Podman Machine... ---" -Color Green
try {
    $machineExists = podman machine list --format "{{.Name}}" | Select-String -Pattern "podman-machine-default"
    if (-not $machineExists) {
        Write-Log "Podman Machine not found. Starting initialization (this may take a long time)..." -Color Yellow
        podman machine init
    }
    
    Write-Log "Starting Podman Machine..." -Color Cyan
    podman machine start
    Write-Log "Podman Machine started successfully." -Color Green
} catch {
    Write-Log "CRITICAL ERROR during Podman Machine setup: $_" -Color Red
    exit 1
}

# --- STEP 3/5: PULL AND ARCHIVE IMAGES (ONE-TIME ONLY) ---

Write-Log "--- [STEP 3/5] Checking for offline image archives... ---" -Color Green

$OllamaArchivePath = Join-Path $ArchiveDir $OllamaArchiveFile
$AppArchivePath = Join-Path $ArchiveDir $AppArchiveFile

if (-not (Test-Path $OllamaArchivePath) -or -not (Test-Path $AppArchivePath)) {
    Write-Log "One or more archives not found. Starting 'Pull & Archive' phase." -Color Yellow
    Write-Log "This requires an internet connection and login ONLY ONCE." -Color Yellow

    try {
        $Username = Read-Host "Enter your Docker Hub username"
        Write-Host "Enter your Docker Hub password:"
        $Password = Read-Host -AsSecureString
        $BSTR = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($Password)
        $PlainTextPassword = [System.Runtime.InteropServices.Marshal]::PtrToStringAuto($BSTR)

        echo $PlainTextPassword | podman login docker.io --username $Username --password-stdin
        
        Write-Log "Pulling Ollama image..." -Color Cyan
        podman pull $OllamaImage
        
        Write-Log "Pulling AnythingLLM image (using corrected name)..." -Color Cyan
        podman pull $AppImage
        
        Write-Log "Saving Ollama to $OllamaArchivePath..." -Color Cyan
        podman save -o $OllamaArchivePath $OllamaImage
        
        Write-Log "Saving AnythingLLM to $AppArchivePath..." -Color Cyan
        podman save -o $AppArchivePath $AppImage
        
        podman logout docker.io
        Write-Log "Archives created successfully. The system is now fully autonomous." -Color Green
    } catch {
        Write-Log "ERROR during 'Pull & Archive' phase: $_" -Color Red
        exit 1
    }
} else {
    Write-Log "All required archives already exist. Skipping 'Pull & Archive' phase." -Color Cyan
}

# --- STEP 4/5: DEPLOY FROM LOCAL DATA ---

Write-Log "--- [STEP 4/5] Deploying system from local archives... ---" -Color Green

try {
    Set-Location $ProjectDir
    
    Write-Log "Performing a full cleanup of any previous installation..." -Color Yellow
    podman compose down --volumes 2>$null | Out-Null

    Write-Log "Loading Ollama from archive..." -Color Cyan
    podman load -i $OllamaArchivePath

    Write-Log "Loading AnythingLLM from archive..." -Color Cyan
    podman load -i $AppArchivePath

} catch {
    Write-Log "ERROR during deployment preparation: $_" -Color Red
    exit 1
}

# --- STEP 5/5: LAUNCH ---

Write-Log "--- [STEP 5/5] Launching the system... ---" -Color Green

try {
    # Using 'podman compose' without the dash
    podman compose up -d
    Write-Log "System launched successfully!" -Color Green
    Write-Log "Please wait 1-2 minutes for services to initialize completely." -Color Yellow
    Write-Log "Then, open your web browser and go to:" -Color Green
    Write-Log "http://localhost:3001" -Color Cyan
} catch {
    Write-Log "CRITICAL ERROR during 'podman compose up': $_" -Color Red
    exit 1
}