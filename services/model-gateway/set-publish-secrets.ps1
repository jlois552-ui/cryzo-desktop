# Sets up publishing on the Convex deployment. The Cloudflare API token comes from a hidden prompt
# and is never echoed, logged or written to disk. The account and zone IDs are looked up with the
# token, so nothing else needs copying. Re-run any time to replace the token.
param(
    # The domain user apps are published under.
    [string]$Domain = 'cryzo.dev',
    # Writes to the Convex production deployment (the one the desktop app uses).
    [switch]$Prod
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$target = @(if ($Prod) { '--prod' })

function Read-Secret([string]$Prompt) {
    $secure = Read-Host -Prompt $Prompt -AsSecureString
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer).Trim() }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
}

$token = Read-Secret 'Cloudflare API token (hidden)'
if (-not $token) { throw 'No token entered; nothing changed.' }
$headers = @{ Authorization = "Bearer $token" }

# Read-only calls that each need one of the token's permissions, so a missing one is named here
# instead of failing on the first publish.
function Test-Cloudflare([string]$Path, [string]$Permission) {
    try { (Invoke-RestMethod "https://api.cloudflare.com/client/v4$Path" -Headers $headers).result }
    catch { throw "Cloudflare refused '$Permission'. Edit the token to add it, then run again." }
}

$accounts = @(Test-Cloudflare '/accounts' 'Account access')
if ($accounts.Count -ne 1) { throw "The token must cover exactly one account; it covers $($accounts.Count)." }
$account = $accounts[0].id
$zones = @(Test-Cloudflare "/zones?name=$Domain" "Zone Read on $Domain")
if ($zones.Count -ne 1) { throw "The token can't see the $Domain zone. Add it under Zone Resources, then run again." }
$zone = $zones[0].id
Test-Cloudflare "/accounts/$account/workers/scripts" 'Workers Scripts' | Out-Null
Test-Cloudflare "/accounts/$account/d1/database" 'D1' | Out-Null
Test-Cloudflare "/accounts/$account/workers/domains?zone_id=$zone" 'Workers Routes' | Out-Null
Test-Cloudflare "/zones/$zone/dns_records?per_page=1" 'DNS' | Out-Null
Write-Host "Cloudflare accepted the token for account '$($accounts[0].name)' and $Domain."

bunx convex env set @target CLOUDFLARE_ACCOUNT_ID $account | Out-Null
bunx convex env set @target CLOUDFLARE_ZONE_ID $zone | Out-Null
bunx convex env set @target CLOUD_APP_DOMAIN $Domain | Out-Null
bunx convex env set @target CLOUDFLARE_API_TOKEN $token | Out-Null
Write-Host "Saved publishing settings on the $(if ($Prod) { 'production' } else { 'development' }) gateway."
