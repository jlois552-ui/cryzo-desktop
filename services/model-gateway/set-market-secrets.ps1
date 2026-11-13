# Sets Marketing's OAuth app credentials on the Convex deployment. Secrets come from hidden prompts
# and are never echoed, logged or written to disk. Pass -Network to set one network; leave a prompt
# empty to keep the current value. MARKET_TOKEN_KEY is generated once and never shown.
param(
    [ValidateSet('x', 'linkedin', 'youtube', 'tiktok', 'meta', 'reddit')][string[]]$Network = @('x', 'linkedin', 'youtube', 'tiktok', 'meta', 'reddit'),
    # Writes to the Convex production deployment.
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

$names = (bunx convex env list @target | ForEach-Object { ($_ -split '=', 2)[0] })
if ($names -notcontains 'MARKET_TOKEN_KEY') {
    # Losing or changing this key makes every stored connection unreadable; channels must then reconnect.
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    bunx convex env set @target MARKET_TOKEN_KEY ([Convert]::ToBase64String($bytes)) | Out-Null
    Write-Host 'Generated MARKET_TOKEN_KEY.'
}

$apps = [ordered]@{
    x        = @('X (OAuth 2.0 Client ID)', 'X_CLIENT_ID', 'X_CLIENT_SECRET')
    linkedin = @('LinkedIn', 'LINKEDIN_CLIENT_ID', 'LINKEDIN_CLIENT_SECRET')
    youtube  = @('YouTube (Google OAuth client)', 'YOUTUBE_CLIENT_ID', 'YOUTUBE_CLIENT_SECRET')
    tiktok   = @('TikTok (client key)', 'TIKTOK_CLIENT_KEY', 'TIKTOK_CLIENT_SECRET')
    # Instagram also uses this app; its Unified Login configuration ID is the META_INSTAGRAM_CONFIG_ID variable.
    meta     = @('Meta (Facebook and Instagram app ID)', 'META_APP_ID', 'META_APP_SECRET')
    reddit   = @('Reddit', 'REDDIT_CLIENT_ID', 'REDDIT_CLIENT_SECRET')
}
foreach ($name in $Network) {
    $label, $idName, $secretName = $apps[$name]
    $id = (Read-Host "$label ID").Trim()
    if ($id) { bunx convex env set @target $idName $id | Out-Null }
    $secret = Read-Secret "$label secret (hidden)"
    if ($secret) { bunx convex env set @target $secretName $secret | Out-Null }
    if ($id -or $secret) { Write-Host "Saved $label." }
}

$base = "$(bunx convex env get @target MARKET_CALLBACK_BASE)".Trim()
if (-not $base) { $base = "$((bunx convex env get @target SITE_URL).Trim())/market" }
Write-Host 'Callback URLs to register in each developer portal:'
foreach ($channel in 'x', 'linkedin', 'youtube', 'tiktok', 'facebook', 'instagram', 'reddit') { Write-Host "  $base/oauth/$channel/callback" }
