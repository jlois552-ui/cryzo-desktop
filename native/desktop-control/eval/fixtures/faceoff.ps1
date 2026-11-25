param([Parameter(Mandatory)][string]$Directory, [Parameter(Mandatory)][string]$Tag, [switch]$Workflow, [switch]$TargetForeground)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class FaceoffDesktop {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
    public struct Point { public int X; public int Y; }
}
'@

function New-Fixture([string]$Title, [int]$Left, [string]$Counter) {
    $form = New-Object Windows.Forms.Form
    $form.Text = $Title
    $form.Size = [Drawing.Size]::new(280, 160)
    $form.StartPosition = 'Manual'
    $form.Location = [Drawing.Point]::new($Left, 60)
    $button = New-Object Windows.Forms.Button
    $button.Text = 'Increment'
    $button.AccessibleName = 'Increment'
    $button.Location = [Drawing.Point]::new(20, 20)
    $button.Size = [Drawing.Size]::new(110, 32)
    $button.Tag = Join-Path $Directory $Counter
    $label = New-Object Windows.Forms.Label
    $label.Text = 'Count 0'
    $label.Location = [Drawing.Point]::new(20, 62)
    $label.AutoSize = $true
    $form.Controls.AddRange(@($button, $label))
    $button.Add_Click({
        param($sender, $eventArgs)
        $count = 1 + [int][IO.File]::ReadAllText($sender.Tag)
        [IO.File]::WriteAllText($sender.Tag, [string]$count)
        $sender.Parent.Controls[1].Text = "Count $count"
    })
    [IO.File]::WriteAllText($button.Tag, '0')
    return $form
}

function New-WorkflowWindow([string]$Title, [int]$Left, [string]$Stem) {
    $form = New-Object Windows.Forms.Form
    $form.Text = $Title
    $form.Size = [Drawing.Size]::new(300, 245)
    $form.StartPosition = 'Manual'
    $form.Location = [Drawing.Point]::new($Left, 60)
    $form.Tag = Join-Path $Directory $Stem
    foreach ($field in @(@{ name = 'Ticket ID'; key = 'ticket'; y = 12 }, @{ name = 'Notes'; key = 'notes'; y = 61 })) {
        $label = New-Object Windows.Forms.Label
        $label.Text = $field.name
        $label.AutoSize = $true
        $label.Location = [Drawing.Point]::new(12, $field.y)
        $form.Controls.Add($label)
        $box = New-Object Windows.Forms.TextBox
        $box.Name = $field.key
        $box.AccessibleName = $field.name
        $box.Location = [Drawing.Point]::new(12, ($field.y + 20))
        $box.Multiline = $field.key -eq 'notes'
        $box.Size = [Drawing.Size]::new(254, $(if ($box.Multiline) { 53 } else { 25 }))
        $box.Add_TextChanged({
            param($control, $eventArgs)
            [IO.File]::AppendAllText(($control.Parent.Tag + '.events'), ("text $($control.Name)" + [Environment]::NewLine))
        })
        $form.Controls.Add($box)
    }
    $review = New-Object Windows.Forms.CheckBox
    $review.Name = 'review'
    $review.Text = 'Reviewed'
    $review.AutoSize = $true
    $review.Location = [Drawing.Point]::new(12, 141)
    $review.Add_CheckedChanged({
        param($control, $eventArgs)
        [IO.File]::AppendAllText(($control.Parent.Tag + '.events'), ('review' + [Environment]::NewLine))
    })
    $form.Controls.Add($review)
    $status = New-Object Windows.Forms.Label
    $status.Name = 'saveStatus'
    $status.Text = 'Draft pending'
    $status.AutoSize = $true
    $status.Location = [Drawing.Point]::new(125, 175)
    $form.Controls.Add($status)
    $save = New-Object Windows.Forms.Button
    $save.Text = 'Save draft'
    $save.Location = [Drawing.Point]::new(12, 169)
    $save.Size = [Drawing.Size]::new(100, 29)
    $save.Add_Click({
        param($control, $eventArgs)
        $parent = $control.Parent
        $record = @{
            ticket = $parent.Controls['ticket'].Text
            notes = $parent.Controls['notes'].Text
            reviewed = $parent.Controls['review'].Checked
        }
        [IO.File]::AppendAllText(($parent.Tag + '.saves'), (($record | ConvertTo-Json -Compress) + [Environment]::NewLine))
        [IO.File]::AppendAllText(($parent.Tag + '.events'), ('save' + [Environment]::NewLine))
        $parent.Controls['saveStatus'].Text = 'Draft saved'
    })
    $form.Controls.Add($save)
    return $form
}

$source = $null
if ($Workflow) {
    $target = New-WorkflowWindow "Cryzo Faceoff $Tag Target" 320 'target'
    $decoy = New-WorkflowWindow "Cryzo Faceoff $Tag Decoy" 620 'decoy'
    $source = New-Object Windows.Forms.Form
    $source.Text = "Cryzo Faceoff $Tag Source"
    $source.Size = [Drawing.Size]::new(297, 155)
    $source.StartPosition = 'Manual'
    $source.Location = [Drawing.Point]::new(20, 60)
    $lines = @("Ticket: CUA-$Tag", 'Summary: Replace scanner label', "Location: Rack $Tag")
    for ($i = 0; $i -lt $lines.Length; $i++) {
        $line = New-Object Windows.Forms.Label
        $line.Text = $lines[$i]
        $line.AutoSize = $true
        $line.Location = [Drawing.Point]::new(12, (12 + $i * 26))
        $source.Controls.Add($line)
    }
    $source.Show()
} else {
    $target = New-Fixture "Cryzo Faceoff $Tag Target" 35 'target.txt'
    $decoy = New-Fixture "Cryzo Faceoff $Tag Decoy" 340 'decoy.txt'
}
$target.Show()
$decoy.Show()
if ($TargetForeground) { $target.Activate() }
else { $decoy.Activate() }
$lastForeground = ''
$lastPosition = ''
$changes = @{ foreground = 0; pointer = 0 }
$timer = New-Object Windows.Forms.Timer
$timer.Interval = 25
$timer.Add_Tick({
    $point = New-Object FaceoffDesktop+Point
    $null = [FaceoffDesktop]::GetCursorPos([ref]$point)
    $foreground = [FaceoffDesktop]::GetForegroundWindow().ToInt64().ToString()
    $position = "$($point.X),$($point.Y)"
    if ($script:lastForeground -ne $foreground) { $changes.foreground++ }
    if ($script:lastPosition -ne $position) { $changes.pointer++ }
    $script:lastForeground = $foreground
    $script:lastPosition = $position
    $state = @{
        foreground = $foreground
        target = $target.Handle.ToInt64().ToString()
        decoy = $decoy.Handle.ToInt64().ToString()
        source = $(if ($source) { $source.Handle.ToInt64().ToString() })
        focusChanges = $changes.foreground
        pointerChanges = $changes.pointer
    }
    $output = Join-Path $Directory 'observation.json'
    try { [IO.File]::WriteAllText($output, ($state | ConvertTo-Json -Compress)) }
    catch [IO.IOException] { }
})
$timer.Start()
[Windows.Forms.Application]::Run($decoy)
