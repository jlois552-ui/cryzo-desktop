# Sets the gateway's provider secrets on the Convex deployment from hidden prompts.
# Values are never echoed, logged or written to disk. Leave a prompt empty to skip that value.
param([switch]$Prod)
$ErrorActionPreference = 'Stop'
# -Prod writes to the Convex production deployment.
$target = @(if ($Prod) { '--prod' })
Set-Location $PSScriptRoot

function Read-Secret([string]$Prompt) {
    $secure = Read-Host -Prompt $Prompt -AsSecureString
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer).Trim() }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
}

$nvidia = Read-Secret 'NVIDIA API key (nvapi-..., hidden)'
if ($nvidia) {
    # A one-token request: the public model list does not verify a key, inference does.
    $body = '{"model":"nvidia/nemotron-3.5-lightning-30b-a3b","messages":[{"role":"user","content":"hi"}],"max_tokens":1}'
    $status = try {
        (Invoke-WebRequest 'https://integrate.api.nvidia.com/v1/chat/completions' -Method Post -UseBasicParsing `
            -Headers @{ Authorization = "Bearer $nvidia" } -ContentType 'application/json' -Body $body).StatusCode
    } catch { [int]$_.Exception.Response.StatusCode }
    if ($status -in 401, 403) { throw "NVIDIA rejected this key (HTTP $status). Create a new key at build.nvidia.com and run again." }
    Write-Host "NVIDIA accepted the key (HTTP $status)."
    bunx convex env set @target NVIDIA_API_KEY $nvidia
}

$clientId = (Read-Host 'Google OAuth client ID (ends in .apps.googleusercontent.com)').Trim()
if ($clientId) { bunx convex env set @target GOOGLE_CLIENT_ID $clientId }
$clientSecret = Read-Secret 'Google OAuth client secret (GOCSPX-..., hidden)'
if ($clientSecret) { bunx convex env set @target GOOGLE_CLIENT_SECRET $clientSecret }

$site = (bunx convex env get @target SITE_URL).Trim()
Write-Host "Gateway health: $((Invoke-RestMethod "$site/health") | ConvertTo-Json -Compress)"
