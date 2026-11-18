# One-time setup for remote control. Run it yourself: it signs in to Cloudflare in your browser.
#  1. Deploys this relay to Cloudflare Workers (free plan, *.workers.dev).
#  2. Generates REMOTE_TICKET_KEY once and pipes it to both the relay and the Convex gateway.
#     The key is never shown, logged or written to disk.
#  3. Sets REMOTE_RELAY_URL (and REMOTE_WEB_URL when you pass -WebUrl) on the gateway.
# Then deploy the gateway (`bunx convex deploy` in services/model-gateway) so the /remote routes exist.
param(
    # Writes to the Convex production deployment.
    [switch]$Prod,
    # Where the remote web app (packages/remote-web) is hosted, e.g. https://cryzo-remote.vercel.app
    [string]$WebUrl
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$target = @(if ($Prod) { '--prod' })

bunx wrangler whoami | Out-Null
if ($LASTEXITCODE -ne 0) { bunx wrangler login }

$deploy = bunx wrangler deploy 2>&1 | Out-String
Write-Host $deploy
$match = [regex]::Match($deploy, 'https://[a-z0-9.-]+\.workers\.dev')
if (-not $match.Success) { throw 'Could not find the relay URL in the deploy output.' }
$relay = $match.Value -replace '^https://', 'wss://'

$gateway = Join-Path $PSScriptRoot '..\model-gateway'
Push-Location $gateway
try {
    $names = (bunx convex env list @target | ForEach-Object { ($_ -split '=', 2)[0] })
} finally { Pop-Location }

if ($names -notcontains 'REMOTE_TICKET_KEY') {
    # Changing this key later only signs everyone's devices out of open relay connections for up to 5 minutes.
    $bytes = New-Object byte[] 48
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $key = [Convert]::ToBase64String($bytes)
    try {
        $key | bunx wrangler secret put REMOTE_TICKET_KEY | Out-Null
        Push-Location $gateway
        try { bunx convex env set @target REMOTE_TICKET_KEY $key | Out-Null } finally { Pop-Location }
    } finally { $key = $null }
    Write-Host 'Generated REMOTE_TICKET_KEY for the relay and the gateway.'
}

Push-Location $gateway
try {
    bunx convex env set @target REMOTE_RELAY_URL "$relay/connect" | Out-Null
    if ($WebUrl) { bunx convex env set @target REMOTE_WEB_URL $WebUrl.TrimEnd('/') | Out-Null }
} finally { Pop-Location }
Write-Host "Relay: $relay/connect"
