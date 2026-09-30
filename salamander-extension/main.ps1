# Salamatrix communicates through stdout using UTF-8 SMX1 frames. Windows PowerShell
# otherwise may inherit the active ANSI/OEM code page, which corrupts names such
# as "domaHD obyvák" before Salamander receives them. Set every relevant
# PowerShell/.NET encoding before making any host call.
$utf8 = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = $utf8
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8

Set-StrictMode -Version 2.0

function Invoke-TuyaTaskbar {
    param([hashtable]$Request)
    $pipe = [IO.Pipes.NamedPipeClientStream]::new(
        '.', 'TuyaSmartTaskbar', [IO.Pipes.PipeDirection]::InOut,
        [IO.Pipes.PipeOptions]::None)
    try {
        $pipe.Connect(2000)
        $encoding = [Text.UTF8Encoding]::new($false)
        $writer = [IO.StreamWriter]::new($pipe, $encoding, 4096, $true)
        $writer.AutoFlush = $true
        $writer.WriteLine(($Request | ConvertTo-Json -Compress -Depth 8))
        $reader = [IO.StreamReader]::new($pipe, $encoding, $false, 4096, $true)
        $line = $reader.ReadLine()
        if ([string]::IsNullOrWhiteSpace($line)) {
            throw 'Tuya Smart Taskbar did not return an IPC response.'
        }
        return (ConvertFrom-Json -InputObject $line)
    } finally {
        if ($null -ne $pipe) { $pipe.Dispose() }
    }
}
function Get-DeviceType { param($Device)
    $category = [string]$Device.category
    if ($category -match 'light|lamp|dj|照明') { return 'light' }
    if ($category -match 'switch|outlet|plug| ổ') { return 'switch' }
    if ($category -match 'fan') { return 'fan' }
    if ($category -match 'air') { return 'air-conditioner' }
    return 'device'
}
function Get-DeviceIcon { param([string]$Type)
    $icon = switch ($Type) {
        'light' { 'icons/light.svg' }
        'switch' { 'icons/switch.svg' }
        'fan' { 'icons/fan.svg' }
        'air-conditioner' { 'icons/air-conditioner.svg' }
        default { 'icons/device.svg' }
    }
    return Join-Path $PSScriptRoot $icon
}
function ConvertFrom-Utf8Hex {
    param([string]$Hex)
    if ([string]::IsNullOrWhiteSpace($Hex)) { return '' }
    $bytes = [byte[]]::new([int]($Hex.Length / 2))
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [Convert]::ToByte($Hex.Substring($index * 2, 2), 16)
    }
    return [Text.Encoding]::UTF8.GetString($bytes)
}

function ConvertTo-DeviceItem { param($Device, $Status)
    $statusRows = @($Status)
    $power = $statusRows | Where-Object { $_.code -in @('switch_led','switch','power','switch_1') } | Select-Object -First 1
    $type = Get-DeviceType $Device
    $online = [bool]$Device.online
    $statusText = if ($online) { 'Online' } else { 'Offline' }
    $isOn = $null -ne $power -and [bool]$power.value
    $powerText = if ($null -eq $power) { 'Unknown' } elseif ($isOn) { 'On' } else { 'Off' }
    $icon = if ($type -eq 'light' -and -not $isOn) { 'icons/light-off.svg' } else { 'icon.svg' }
    $iconDark = if ($type -eq 'light' -and -not $isOn) { 'icons/light-off-dark.svg' } else { 'icon-dark.svg' }
    $name = [string]$Device.name
    return @{ id=($type + '-' + [string]$Device.id); name=$name; compactName=$name; directory=$false; enabled=$online; icon=$icon; iconDark=$iconDark; columns=@{ category=$type; status=$statusText; power=$powerText; deviceId=[string]$Device.id } }
}
$handler = [string]$Salamander.command_handler
if ($handler -eq 'listDevices') {
    $result = Invoke-TuyaTaskbar @{operation='list'}
    if (-not $result.ok) { throw [string]$result.error }
    $statuses = @{}
    foreach ($entry in $result.statuses.PSObject.Properties) { $statuses[$entry.Name] = @($entry.Value) }
    $items = New-Object 'System.Collections.Generic.List[hashtable]'
    foreach ($device in @($result.devices)) { $items.Add((ConvertTo-DeviceItem $device $statuses[[string]$device.id])) }
    [void]$Salamander.file_system.AddItems($items.ToArray())
    return
}
$item = $Salamander.invocation.item
if ($null -eq $item) { return }
$deviceId = ([string]$item.id) -replace '^(light|switch|fan|air-conditioner|device)-',''
$deviceName = [string]$item.name
if ($handler -eq 'openDevice' -and ([string]$item.id).StartsWith('light-')) { [void](Invoke-TuyaTaskbar @{operation='command';action='custom';deviceId=$deviceId}); return }
$action = $handler
$percent = $null
if ($handler -eq 'daylight') { $action = 'profile'; $percent = 70 }
if ($handler -eq 'evening') { $action = 'profile'; $percent = 30 }
if ($handler -eq 'nightlight') { $action = 'profile'; $percent = 5 }
$request = @{operation='command';action=$action;deviceId=$deviceId}
if ($null -ne $percent) { $request.percent = $percent }
$result = Invoke-TuyaTaskbar $request
if (-not $result.ok) { throw [string]$result.error }
