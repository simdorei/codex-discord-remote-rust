function Get-SidebarRightBoundary {
  param($WindowRect)
  return [double]($WindowRect.Left + [Math]::Max(320, [Math]::Floor($WindowRect.Width * 0.42)))
}

function Is-SidebarElement {
  param($Element, $WindowRect)
  try {
$rect = $Element.Current.BoundingRectangle
if ($rect.Width -le 1 -or $rect.Height -le 1) { return $false }
if ($rect.Top -lt ($WindowRect.Top + 40)) { return $false }
if ($rect.Left -gt (Get-SidebarRightBoundary $WindowRect)) { return $false }
if ($rect.Right -lt ($WindowRect.Left + 12)) { return $false }
if ($rect.Bottom -gt ($WindowRect.Bottom - 36) -and $rect.Top -gt ($WindowRect.Top + ($WindowRect.Height * 0.65))) {
  return $false
}
return $true
  } catch {
Add-SidebarActivationError 'sidebar_element_rect' $_
return $false
  }
}

function Find-SidebarElementByNameContains {
  param($Elements, $WindowRect, [string]$Needle)
  if (-not $Needle) { return $null }
  $types = @('ControlType.ListItem', 'ControlType.Button', 'ControlType.Text')
  foreach ($controlTypeName in $types) {
foreach ($el in $Elements) {
  if ($el.Current.ControlType.ProgrammaticName -ne $controlTypeName) { continue }
  if (-not (Is-SidebarElement $el $WindowRect)) { continue }
  $name = Normalize-Name $el.Current.Name
  if ($name -and $name.Contains($Needle)) {
    return $el
  }
}
  }
  return $null
}

function Get-VisibleSidebarNames {
  param($Elements, $WindowRect)
  $names = New-Object System.Collections.Generic.List[string]
  foreach ($el in $Elements) {
$type = $el.Current.ControlType.ProgrammaticName
if ($type -notin @('ControlType.ListItem', 'ControlType.Button', 'ControlType.Text')) { continue }
if (-not (Is-SidebarElement $el $WindowRect)) { continue }
$name = Normalize-Name $el.Current.Name
if (-not $name) { continue }
if (-not $names.Contains($name)) { [void]$names.Add($name) }
  }
  return ($names -join ' | ')
}

function Find-SidebarToggleButton {
  param($Elements, $WindowRect)
  $koreanSidebar = -join @([char]0xC0AC, [char]0xC774, [char]0xB4DC, [char]0xBC14)
  foreach ($el in $Elements) {
if ($el.Current.ControlType.ProgrammaticName -ne 'ControlType.Button') { continue }
$name = Normalize-Name $el.Current.Name
if (-not $name) { continue }
if (-not ($name -match '(?i)sidebar' -or $name.Contains($koreanSidebar))) { continue }
try {
  $rect = $el.Current.BoundingRectangle
  if ($rect.Left -gt ($WindowRect.Left + 72)) { continue }
  if ($rect.Top -gt ($WindowRect.Top + 48)) { continue }
  if ($rect.Width -le 1 -or $rect.Height -le 1) { continue }
  return $el
} catch { Add-SidebarActivationError 'sidebar_toggle_rect' $_ }
  }
  return $null
}

function SidebarToggleLooksClosed {
  param([string]$Name)
  if (-not $Name) { return $false }
  if ($Name -match '(?i)(show|open).{0,24}sidebar|sidebar.{0,24}(show|open)') { return $true }
  $koreanSidebar = -join @([char]0xC0AC, [char]0xC774, [char]0xB4DC, [char]0xBC14)
  $koreanShow = -join @([char]0xBCF4, [char]0xAE30)
  $koreanOpen = -join @([char]0xC5F4, [char]0xAE30)
  if ($Name.Contains($koreanSidebar) -and ($Name.Contains($koreanShow) -or $Name.Contains($koreanOpen))) { return $true }
  return $false
}

function Ensure-SidebarOpen {
  param($Window, $Elements, $WindowRect)
  $toggle = Find-SidebarToggleButton $Elements $WindowRect
  if (-not $toggle) { return $Elements }
  $name = Normalize-Name $toggle.Current.Name
  if (-not (SidebarToggleLooksClosed $name)) { return $Elements }
  if (Invoke-Or-Click $toggle) {
Start-Sleep -Milliseconds 420
return Refresh-AllElements $Window
  }
  return $Elements
}

function Has-TerminalTabs {
  param($Elements)
  foreach ($el in $Elements) {
if ($el.Current.ControlType.ProgrammaticName -ne 'ControlType.ListItem') { continue }
$name = Normalize-Name $el.Current.Name
if ($name -match '^Terminal\s+\d+') {
  return $true
}
  }
  return $false
}

function Try-ExpandElement {
  param($Element)
  if (-not $Element) { return $false }
  $pattern = $null
  if ($Element.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$pattern)) {
try {
  if ($pattern.Current.ExpandCollapseState -eq [System.Windows.Automation.ExpandCollapseState]::Collapsed) {
    $pattern.Expand()
    return $true
  }
  if ($pattern.Current.ExpandCollapseState -eq [System.Windows.Automation.ExpandCollapseState]::Expanded) {
    return $true
  }
} catch { Add-SidebarActivationError 'expand_element' $_ }
  }
  return $false
}

function Expand-ProjectSection {
  param($Window, $Elements, $WindowRect, [string]$ProjectName)
  if (-not $ProjectName) { return $Elements }
  $projectElement = Find-SidebarElementByNameContains $Elements $WindowRect $ProjectName
  if (-not $projectElement) { return $Elements }

  if (Try-ExpandElement $projectElement) {
Start-Sleep -Milliseconds 220
return Refresh-AllElements $Window
  }

  $buttonCondition = New-Object System.Windows.Automation.PropertyCondition(
[System.Windows.Automation.AutomationElement]::ControlTypeProperty,
[System.Windows.Automation.ControlType]::Button
  )
  $buttons = $projectElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
  for ($i = 0; $i -lt $buttons.Count; $i++) {
$btn = $buttons.Item($i)
if (Try-ExpandElement $btn) {
  Start-Sleep -Milliseconds 220
  return Refresh-AllElements $Window
}
  }

  if (Invoke-Or-Click $projectElement) {
Start-Sleep -Milliseconds 260
return Refresh-AllElements $Window
  }

  return $Elements
}

function Stabilize-Ui {
  param($Window, $Elements, $WindowRect)
  for ($i = 0; $i -lt 2; $i++) {
Send-Key 0x1B
Start-Sleep -Milliseconds 120
  }
  $Elements = Refresh-AllElements $Window
  $Elements = Ensure-SidebarOpen $Window $Elements $WindowRect
  $sidebarNames = Get-VisibleSidebarNames $Elements $WindowRect

  if ((-not $sidebarNames) -and (Has-TerminalTabs $Elements)) {
Send-Hotkey @(0x11, 0x4A)
Start-Sleep -Milliseconds 350
$Elements = Refresh-AllElements $Window
$Elements = Ensure-SidebarOpen $Window $Elements $WindowRect
$sidebarNames = Get-VisibleSidebarNames $Elements $WindowRect
  }

  if (-not $sidebarNames) {
Send-Hotkey @(0x11, 0x42)
Start-Sleep -Milliseconds 350
$Elements = Refresh-AllElements $Window
$Elements = Ensure-SidebarOpen $Window $Elements $WindowRect
  }

  return $Elements
}

function Get-SidebarAnchorPoint {
  param($WindowRect)
  $x = [int]($WindowRect.Left + [Math]::Min(260, [Math]::Max(150, [Math]::Floor($WindowRect.Width * 0.16))))
  $y = [int]($WindowRect.Top + [Math]::Min($WindowRect.Height - 200, [Math]::Max(220, [Math]::Floor($WindowRect.Height * 0.38))))
  return @{ X = $x; Y = $y }
}

function Scroll-Sidebar {
  param($WindowRect, [int]$Delta)
  $anchor = Get-SidebarAnchorPoint $WindowRect
  [void][Native]::SetCursorPos($anchor.X, $anchor.Y)
  Start-Sleep -Milliseconds 50
  [Native]::mouse_event(0x0800, 0, 0, $Delta, [UIntPtr]::Zero)
}

function Find-ThreadWithScroll {
  param($Window, $Elements, $WindowRect, [string]$ThreadName, [string]$ProjectName)
  $candidate = Find-SidebarElementByNameContains $Elements $WindowRect $ThreadName
  if ($candidate) { return @{ Elements = $Elements; Target = $candidate } }

  for ($i = 0; $i -lt 6; $i++) {
Scroll-Sidebar $WindowRect 120
Start-Sleep -Milliseconds 120
  }
  $Elements = Refresh-AllElements $Window
  $Elements = Expand-ProjectSection $Window $Elements $WindowRect $ProjectName
  $candidate = Find-SidebarElementByNameContains $Elements $WindowRect $ThreadName
  if ($candidate) { return @{ Elements = $Elements; Target = $candidate } }

  for ($step = 0; $step -lt 18; $step++) {
Scroll-Sidebar $WindowRect -240
Start-Sleep -Milliseconds 150
$Elements = Refresh-AllElements $Window
$Elements = Expand-ProjectSection $Window $Elements $WindowRect $ProjectName
$candidate = Find-SidebarElementByNameContains $Elements $WindowRect $ThreadName
if ($candidate) {
  return @{ Elements = $Elements; Target = $candidate }
}
  }

  return @{ Elements = $Elements; Target = $null }
}
