<#
Computer-use race: the same 8-step job for two agents (for example Cryzo and Codex with the same
model), timed from when you send the prompt to when the agent saves its answers file, then checked
against the real end state.

  powershell -ExecutionPolicy Bypass -File race.ps1 setup cryzo   # builds the files, copies the prompt
  powershell -ExecutionPolicy Bypass -File race.ps1 check cryzo   # time and score
  powershell -ExecutionPolicy Bypass -File race.ps1 setup codex
  powershell -ExecutionPolicy Bypass -File race.ps1 check codex
  powershell -ExecutionPolicy Bypass -File race.ps1 stop          # stops the little web server

Each agent gets its own folder and file names, so Notepad's restored tabs can't mix the runs up.
#>
param(
  [Parameter(Mandatory)][ValidateSet('setup', 'check', 'stop')][string]$Mode,
  [string]$Name = 'cryzo',
  [int]$Port = 8765
)

$ErrorActionPreference = 'Stop'
$base = Join-Path $env:USERPROFILE 'Documents\computer-use-race'
$root = Join-Path $base $Name
$web = Join-Path $base 'web'
$serverPid = Join-Path $base 'server.pid'

function Stop-Server {
  if (Test-Path $serverPid) {
    Stop-Process -Id ([int](Get-Content $serverPid)) -Force -ErrorAction SilentlyContinue
    Remove-Item $serverPid -Force
  }
}

if ($Mode -eq 'stop') { Stop-Server; 'Web server stopped.'; return }

if ($Mode -eq 'setup') {
  if (Test-Path $root) { Remove-Item $root -Recurse -Force }
  New-Item -ItemType Directory -Force -Path (Join-Path $root 'inbox'), $web | Out-Null
  'a', 'b' | ForEach-Object { Set-Content -Path (Join-Path $root "inbox\$_.txt") -Value "$_ file" }
  Set-Content -Path (Join-Path $root 'inbox\keep.md') -Value 'keep me'
  [IO.File]::WriteAllText((Join-Path $root "draft-$Name.txt"), "The meeting is on Monday.`r`nBring the slides.`r`n")
  $rows = @(('Desk Chair', '$129.00'), ('Blue Lamp', '$34.75'), ('Red Lamp', '$36.20'), ('Bookshelf', '$89.99'), ('Blue Rug', '$64.10')) |
    ForEach-Object { "<tr><td>$($_[0])</td><td>$($_[1])</td></tr>" }
  Set-Content -Encoding UTF8 -Path (Join-Path $web 'prices.html') -Value "<!doctype html><html><head><title>Prices</title></head><body style='font-family:sans-serif;margin:40px'><h1>Store prices</h1><table border='1' cellpadding='6'><tr><th>Item</th><th>Price</th></tr>$($rows -join '')</table></body></html>"
  Set-Content -Encoding UTF8 -Path (Join-Path $web 'code.html') -Value "<!doctype html><html><head><title>Confirmation</title></head><body style='font-family:sans-serif;margin:40px'><h1>Order placed</h1><p>Your confirmation code is <strong>QX-7341</strong>.</p></body></html>"

  # A local web page both agents can reach; extensions often can't open file:// pages.
  $listening = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
  if (-not $listening) {
    $python = (Get-Command python -ErrorAction SilentlyContinue).Source
    if (-not $python) { throw 'Python is needed for the little web server (python -m http.server).' }
    $server = Start-Process -FilePath $python -ArgumentList '-m', 'http.server', $Port, '--bind', '127.0.0.1' -WorkingDirectory $web -WindowStyle Hidden -PassThru
    Set-Content -Path $serverPid -Value $server.Id
    Start-Sleep -Seconds 1
  }

  $prompt = @"
Use computer use for every step: control the apps on screen. Don't run shell commands and don't write files directly. Do these in order:
1. Open Calculator, work out 847 x 293 by clicking its buttons, note the result, and close Calculator.
2. Open Notepad and type exactly these two lines:
Race line one
Second line: 42 apples
Save it as $root\notes-$Name.txt and close that tab.
3. Open $root\draft-$Name.txt in Notepad, change "Monday" to "Friday", save it, and close that tab.
4. In File Explorer, open $root\inbox, create a folder named Archive, and move a.txt and b.txt into it. Leave keep.md where it is.
5. In the Settings app, find this PC's Windows edition and its full OS build number, then close Settings.
6. In a web browser, open http://127.0.0.1:$Port/prices.html and note the price of the Blue Lamp.
7. Open http://127.0.0.1:$Port/code.html and note the confirmation code.
8. Last, in Notepad, create $root\answers-$Name.txt with four lines: the Calculator result, the Windows edition and build, the Blue Lamp price, and the confirmation code. Save it and close that tab. Then reply DONE.
"@
  Set-Clipboard -Value $prompt
  $prompt
  ''
  'The prompt above is on your clipboard.'
  'Paste it into the agent, and press Enter here at the moment you send it.'
  Read-Host | Out-Null
  Set-Content -Path (Join-Path $root 'start.txt') -Value (Get-Date).ToString('o')
  "Started at $(Get-Date -Format 'HH:mm:ss'). When the agent says DONE, run: race.ps1 check $Name"
  return
}

# check
if (-not (Test-Path (Join-Path $root 'start.txt'))) { throw "No race set up for $Name. Run: race.ps1 setup $Name" }
$start = [datetime](Get-Content (Join-Path $root 'start.txt'))
$answersFile = Join-Path $root "answers-$Name.txt"
$read = { param($path) if (Test-Path $path) { ([IO.File]::ReadAllText($path) -replace "^﻿", '' -replace "`r`n", "`n").Trim() } else { $null } }
$version = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$build = "$($version.CurrentBuild).$($version.UBR)"
$edition = (Get-CimInstance Win32_OperatingSystem).Caption -replace '^Microsoft Windows \d+ ', ''
$answers = & $read $answersFile
$inbox = Join-Path $root 'inbox'

$checks = [ordered]@{
  '1 Calculator result'       = [bool]($answers -match '248,?171')
  '2 New Notepad file'        = (& $read (Join-Path $root "notes-$Name.txt")) -ceq "Race line one`nSecond line: 42 apples"
  '3 Edited file'             = (& $read (Join-Path $root "draft-$Name.txt")) -ceq "The meeting is on Friday.`nBring the slides."
  '4 Explorer folder + move'  = (Test-Path "$inbox\Archive\a.txt") -and (Test-Path "$inbox\Archive\b.txt") -and -not (Test-Path "$inbox\a.txt") -and -not (Test-Path "$inbox\b.txt") -and (Test-Path "$inbox\keep.md")
  "5 Settings ($edition $build)" = [bool]($answers -and $answers.Contains($build) -and $answers.ToLower().Contains($edition.ToLower()))
  '6 Blue Lamp price'         = [bool]($answers -match '34\.75')
  '7 Confirmation code'       = [bool]($answers -match 'QX-7341')
  '8 Answers file saved'      = $null -ne $answers
}
''
"Computer-use race: $Name"
$checks.GetEnumerator() | ForEach-Object { '  {0}  {1}' -f $(if ($_.Value) { 'PASS' } else { 'FAIL' }), $_.Key }
$passed = @($checks.Values | Where-Object { $_ }).Count
if (Test-Path $answersFile) {
  $seconds = ((Get-Item $answersFile).LastWriteTime - $start).TotalSeconds
  "  Score: $passed/8   Time: {0:N0} s ({1:N1} min), from sending the prompt to saving the answers" -f $seconds, ($seconds / 60)
} else {
  "  Score: $passed/8   Time: not finished (no answers file yet)"
}
