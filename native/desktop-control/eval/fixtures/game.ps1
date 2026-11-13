# A window that paints its text itself, like a game or canvas: UI Automation sees no labels in it,
# so reading it needs OCR.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Cryzo Eval Game'
$form.Width = 720
$form.Height = 420
$form.StartPosition = 'CenterScreen'
$form.BackColor = [System.Drawing.Color]::FromArgb(18, 22, 40)
$form.Add_Paint({
  param($sender, $event)
  $big = New-Object System.Drawing.Font('Segoe UI', 34, [System.Drawing.FontStyle]::Bold)
  $small = New-Object System.Drawing.Font('Segoe UI', 22)
  $event.Graphics.DrawString('Score 4200', $big, [System.Drawing.Brushes]::Gold, 60, 70)
  $event.Graphics.DrawString('Level 7', $small, [System.Drawing.Brushes]::White, 64, 160)
  $event.Graphics.DrawString('Press Start to play', $small, [System.Drawing.Brushes]::LightGray, 64, 230)
})
[System.Windows.Forms.Application]::Run($form)
