# Copies cryzo.me's Composio settings (API key and per-network auth config IDs) from its Vercel
# project into this Convex deployment, so Marketing uses the same Composio project and connections.
# Values are written to a temporary file by the Vercel CLI, copied, and the file is deleted.
# Nothing is printed. Usage: .\copy-composio-env.ps1 -CryzoMe <path to a linked cryzo-v11 checkout>
param([Parameter(Mandatory = $true)][string]$CryzoMe)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not (Test-Path (Join-Path $CryzoMe '.vercel\project.json'))) {
    throw "$CryzoMe is not linked to Vercel. Run 'vercel link' there first."
}
$names = 'COMPOSIO_API_KEY', 'COMPOSIO_TWITTER_AUTH_CONFIG_ID', 'COMPOSIO_FACEBOOK_AUTH_CONFIG_ID',
    'COMPOSIO_INSTAGRAM_AUTH_CONFIG_ID', 'COMPOSIO_YOUTUBE_AUTH_CONFIG_ID', 'COMPOSIO_TIKTOK_AUTH_CONFIG_ID',
    'COMPOSIO_REDDIT_AUTH_CONFIG_ID'
$file = Join-Path ([IO.Path]::GetTempPath()) ("cryzo-me-" + [guid]::NewGuid() + ".env")
try {
    Push-Location $CryzoMe
    try { vercel env pull $file --environment=production --yes | Out-Null } finally { Pop-Location }
    $values = @{}
    foreach ($line in Get-Content -LiteralPath $file) {
        if ($line -match '^([A-Z0-9_]+)=(.*)$' -and $names -contains $Matches[1]) {
            $values[$Matches[1]] = $Matches[2].Trim().Trim('"')
        }
    }
    foreach ($name in $names) {
        if ($values[$name]) {
            bunx convex env set $name $values[$name] | Out-Null
            Write-Host "Set $name"
        } else {
            Write-Host "Not set on cryzo.me: $name"
        }
    }
} finally {
    Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue
}
Write-Host 'Done. X and TikTok need their auth config IDs before those channels can connect.'
