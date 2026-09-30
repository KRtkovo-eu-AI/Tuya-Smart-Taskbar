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
    $name = [string]$Device.name
    $typeText = ($category + ' ' + $name).ToLowerInvariant()

    # Tuya category codes: dj = light, fs = fan, cl/clkg = curtain.
    # Curtain switches use the same switch-style icon in this extension.
    if ($typeText -match '(^|[^a-z])(dj|light|lamp|照明)([^a-z]|$)') { return 'light' }
    if ($typeText -match '(^|[^a-z])(fs|fan)([^a-z]|$)') { return 'fan' }
    if ($typeText -match 'curtain|clkg|(^|[^a-z])cl([^a-z]|$)|switch|outlet|plug|(^|[^a-z])kg([^a-z]|$)| ổ') { return 'switch' }
    if ($typeText -match 'air[- ]?condition|(^|[^a-z])kt([^a-z]|$)') { return 'air-conditioner' }
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
    return $icon
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

function ConvertTo-DeviceItem { param($Device, $Status, [bool]$StatusError = $false)
    $statusRows = @($Status)
    $hasStatus = $statusRows.Count -gt 0 -and -not $StatusError
    $power = $statusRows | Where-Object { $_.code -in @('switch_led','switch','power','switch_1') } | Select-Object -First 1
    $type = Get-DeviceType $Device
    $online = [bool]$Device.online
    $statusText = if (-not $hasStatus) { 'Error' } elseif ($online) { 'Online' } else { 'Offline' }
    $isOn = $null -ne $power -and [bool]$power.value
    $powerText = if (-not $hasStatus) { 'Error' } elseif ($null -eq $power) { 'Unknown' } elseif ($isOn) { 'On' } else { 'Off' }
    if (-not $hasStatus) {
        $icon = 'icons/error.svg'
        $iconDark = 'icons/error.svg'
    } elseif ($type -eq 'light') {
        $icon = if ($isOn) { 'icons/light.svg' } else { 'icons/light-off.svg' }
        $iconDark = if ($isOn) { 'icons/light.svg' } else { 'icons/light-off-dark.svg' }
    } else {
        $icon = Get-DeviceIcon $type
        $iconDark = $icon
    }
    $name = [string]$Device.name
    return @{ id=('device-' + $type + '-' + [string]$Device.id); name=$name; compactName=$name; directory=$false; enabled=$online; icon=$icon; iconDark=$iconDark; columns=@{ category=$type; status=$statusText; power=$powerText; deviceId=[string]$Device.id } }
}
$handler = [string]$Salamander.command_handler
if ($handler -eq 'listDevices') {
    $result = Invoke-TuyaTaskbar @{operation='list'}
    if (-not $result.ok) { throw [string]$result.error }

    $statuses = @{}
    if ($null -ne $result.statuses -and $result.statuses.PSObject.Properties) {
        foreach ($entry in @($result.statuses.PSObject.Properties)) {
            $statuses[[string]$entry.Name] = @($entry.Value)
        }
    }
    # Use an explicit array of current device IDs. Do not infer errors from the
    # shape of a deserialized PSCustomObject, which caused all devices to turn
    # red after a previously failing device was removed.
    $statusErrorIds = @()
    if ($null -ne $result.statusErrorIds) {
        $statusErrorIds = @($result.statusErrorIds | ForEach-Object { [string]$_ })
    } elseif ($null -ne $result.statusErrors -and $result.statusErrors.PSObject.Properties) {
        $statusErrorIds = @($result.statusErrors.PSObject.Properties | ForEach-Object { [string]$_.Name })
    }

    $devices = if ($null -eq $result.devices) { @() } else { @($result.devices) }
    $items = New-Object 'System.Collections.Generic.List[hashtable]'
    foreach ($device in $devices) {
        try {
            $deviceId = [string]$device.id
            $status = if ($statuses.ContainsKey($deviceId)) { $statuses[$deviceId] } else { @() }
            $statusError = $statusErrorIds -contains $deviceId
            $items.Add((ConvertTo-DeviceItem $device $status $statusError))
        } catch {
            $fallbackId = [string]$device.id
            if ([string]::IsNullOrWhiteSpace($fallbackId)) { continue }
            $fallbackName = [string]$device.name
            if ([string]::IsNullOrWhiteSpace($fallbackName)) { $fallbackName = $fallbackId }
            $items.Add(@{
                id=('device-device-' + $fallbackId); name=$fallbackName; compactName=$fallbackName
                directory=$false; enabled=([bool]$device.online); icon='icons/error.svg'; iconDark='icons/error.svg'
                columns=@{ category='device'; status='Error'; power='Error'; deviceId=$fallbackId }
            })
        }
    }

    # Publish devices before processing rooms. A malformed room must never make
    # the whole filesystem empty.
    $rootDeviceItems = @($items.ToArray())
    $roomsById = @{}
    $roomItems = New-Object 'System.Collections.Generic.List[hashtable]'
    $rooms = if ($null -eq $result.rooms) { @() } else { @($result.rooms) }
    foreach ($room in $rooms) {
        try {
            $roomId = [string]$room.id
            if ([string]::IsNullOrWhiteSpace($roomId)) { continue }
            $roomsById[$roomId] = $room
            $roomDeviceIds = @()
            if ($null -ne $room.device_ids) {
                $roomDeviceIds = @($room.device_ids | ForEach-Object { [string]$_ })
            }
            if ($roomDeviceIds.Count -eq 0 -and $null -ne $room.devices) {
                $roomDeviceIds = @($room.devices | ForEach-Object { [string]$_.id })
            }
            $roomName = [string]$room.name
            $roomItemId = if ($roomDeviceIds.Count -eq 0) { 'empty-room-' + $roomId } else { 'room-' + $roomId }
            $roomItems.Add(@{
                id=$roomItemId; name=($roomName + ' (' + $roomDeviceIds.Count + ' devices)')
                compactName=($roomName + ' (' + $roomDeviceIds.Count + ' devices)')
                directory=$true; enabled=$true; icon='icons/room.svg'; iconDark='icons/room-dark.svg'
                columns=@{ category='room'; status=''; power=''; deviceId=$roomId }
            })
        } catch {
            continue
        }
    }

    # Salamatrix supplies the current filesystem path while listing. The root
    # path contains only the filesystem id; a room path adds room-<id>.
    $path = try { [string]$Salamander.invocation.path } catch { '' }
    $pathParts = @($path -split '[\\/]' | Where-Object {
        -not [string]::IsNullOrWhiteSpace([string]$_)
    })
    $roomPart = @($pathParts | Where-Object { [string]$_ -match '^(room|empty-room)-' } | Select-Object -First 1)
    if ($roomPart.Count -gt 0) {
        $roomId = ([string]$roomPart[0]) -replace '^(room|empty-room)-', ''
        $room = $roomsById[$roomId]
        if ($null -eq $room) { return }
        $childIds = @($room.device_ids | ForEach-Object { [string]$_ })
        $childItems = @($items | Where-Object { $childIds -contains ([string]$_.columns.deviceId) })
        [void]$Salamander.file_system.AddItems($childItems)
    } else {
        $rootItems = @($roomItems.ToArray()) + $rootDeviceItems
        [void]$Salamander.file_system.AddItems($rootItems)
    }
    return
}
$item = $Salamander.invocation.item
if ($null -eq $item) { return }
$itemId = [string]$item.id
if ($itemId -match '^(room|empty-room)-') {
    if ($handler -notin @('roomToggle', 'roomDaylight', 'roomEvening', 'roomNightlight')) { return }
    $roomId = $itemId -replace '^(room|empty-room)-', ''
    $list = Invoke-TuyaTaskbar @{operation='list'}
    if (-not $list.ok) { throw [string]$list.error }
    $room = @($list.rooms) | Where-Object { [string]$_.id -eq $roomId } | Select-Object -First 1
    if ($null -eq $room) { throw "Room not found: $roomId" }
    $roomDeviceIds = @($room.device_ids | ForEach-Object { [string]$_ })
    $lights = @($list.devices | Where-Object {
        $id = [string]$_.id
        $category = [string]$_.category
        $name = [string]$_.name
        $roomDeviceIds -contains $id -and (($category + ' ' + $name) -match '(?i)(^|[^a-z])(dj|light|lamp)([^a-z]|$)')
    })
    foreach ($light in $lights) {
        $action = if ($handler -eq 'roomToggle') { 'toggle' } else { 'profile' }
        $request = @{operation='command'; action=$action; deviceId=[string]$light.id}
        if ($action -eq 'profile') {
            $request.percent = switch ($handler) {
                'roomDaylight' { 70 }
                'roomEvening' { 30 }
                default { 5 }
            }
        }
        $result = Invoke-TuyaTaskbar $request
        if (-not $result.ok) { throw [string]$result.error }
    }
    return
}
$deviceId = $itemId -replace '^device-(light|switch|fan|air-conditioner|device)-',''
$deviceName = [string]$item.name
if ($handler -eq 'openDevice' -and $itemId.StartsWith('device-light-')) { [void](Invoke-TuyaTaskbar @{operation='command';action='custom';deviceId=$deviceId}); return }
$action = $handler
$percent = $null
if ($handler -eq 'daylight') { $action = 'profile'; $percent = 70 }
if ($handler -eq 'evening') { $action = 'profile'; $percent = 30 }
if ($handler -eq 'nightlight') { $action = 'profile'; $percent = 5 }
$request = @{operation='command';action=$action;deviceId=$deviceId}
if ($null -ne $percent) { $request.percent = $percent }
$result = Invoke-TuyaTaskbar $request
if (-not $result.ok) { throw [string]$result.error }





