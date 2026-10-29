param([switch]$NoLaunch)
$ErrorActionPreference = 'Stop'
$taskRepo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskSource = Join-Path $PSScriptRoot 'target/debug/cryzo-desktop.exe'
if (-not (Test-Path -LiteralPath $taskSource)) { throw 'Build the desktop executable first.' }
$taskInstall = Join-Path $env:LOCALAPPDATA 'Programs/CryzoDevelopment'
$null = New-Item -ItemType Directory -Path $taskInstall -Force
$taskExe = Join-Path $taskInstall 'Cryzo.exe'
if (Get-Process -Name Cryzo -ErrorAction SilentlyContinue | Where-Object Path -EQ $taskExe) {
    if (-not $NoLaunch) { throw 'Close Cryzo before updating and launching it, or use -NoLaunch to update the next launch.' }
    # Windows permits renaming a loaded image. Keep it beside the new executable;
    # the active session continues and the normal shortcut uses the new build next time.
    $taskRunningBackup = Join-Path $taskInstall ('Cryzo.running.' + (Get-Date -Format 'yyyyMMddHHmmss') + '.exe')
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($taskRunningBackup)) -ne [IO.Path]::GetFullPath($taskInstall)) { throw 'Invalid installation backup path.' }
    Move-Item -LiteralPath $taskExe -Destination $taskRunningBackup
} elseif (Test-Path -LiteralPath $taskExe) {
    Copy-Item -LiteralPath $taskExe -Destination (Join-Path $taskInstall 'Cryzo.previous.exe') -Force
}
Copy-Item -LiteralPath $taskSource -Destination $taskExe -Force
Copy-Item -LiteralPath (Join-Path $taskRepo 'packages/desktop-tauri/src-tauri/icons/icon.ico') -Destination (Join-Path $taskInstall 'Cryzo.ico') -Force
$taskGcc = Join-Path $env:LOCALAPPDATA 'Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin'
if (Test-Path -LiteralPath (Join-Path $taskGcc 'objdump.exe')) {
    $taskImports = & (Join-Path $taskGcc 'objdump.exe') -p $taskExe | Select-String 'DLL Name:\s*(.+)$'
    foreach ($taskImport in $taskImports) {
        $taskDll = Join-Path $taskGcc $taskImport.Matches[0].Groups[1].Value.Trim()
        if (Test-Path -LiteralPath $taskDll) {
            $taskInstalledDll = Join-Path $taskInstall ([IO.Path]::GetFileName($taskDll))
            if ((Test-Path -LiteralPath $taskInstalledDll) -and ((Get-FileHash -LiteralPath $taskDll).Hash -eq (Get-FileHash -LiteralPath $taskInstalledDll).Hash)) { continue }
            Copy-Item -LiteralPath $taskDll -Destination $taskInstall -Force
        }
    }
}
@'
Cryzo Development - local Tauri/Rust test build

The interface uses the original OpenCode application with Cryzo branding.
Open a local project, connect a supported API-key provider, and choose a model.
Keys stay in the operating system credential store. File writes and agent
commands require approval for the exact operation. The terminal is interactive.

This is an internal test build. Cryzo Free needs its hosted gateway and Google
sign-in configuration. Personal NVIDIA, OpenRouter and other supported provider
keys can be connected through Manage models > Connect provider. Special provider
authentication, MCP,
SSH/WSL, compaction/reverts, Market, Team and Store distribution remain pending.
No signed public release is implied. Your previous executable is preserved
beside Cryzo.exe. Existing application data is retained.
'@ | Set-Content -LiteralPath (Join-Path $taskInstall 'READ-ME.txt') -Encoding utf8
$taskShell = New-Object -ComObject WScript.Shell
foreach ($taskFolder in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
    $taskLink = $taskShell.CreateShortcut((Join-Path $taskFolder 'Cryzo Development.lnk'))
    $taskLink.TargetPath = $taskExe
    $taskLink.WorkingDirectory = $taskInstall
    $taskLink.IconLocation = Join-Path $taskInstall 'Cryzo.ico'
    $taskLink.Description = 'Cryzo - original OpenCode interface with Tauri and Rust'
    $taskLink.Save()
}
if (-not $NoLaunch) {
    $taskApp = Start-Process -FilePath $taskExe -WorkingDirectory $taskInstall -PassThru
    [ordered]@{ executable=$taskExe; pid=$taskApp.Id } | ConvertTo-Json
}
if ($NoLaunch) { [ordered]@{ executable=$taskExe; updatedForNextLaunch=$true } | ConvertTo-Json }
