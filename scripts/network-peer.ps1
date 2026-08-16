param(
    [ValidateRange(1024, 65535)]
    [int]$Port = 4450
)

$ErrorActionPreference = 'Stop'
$gatewayIp = [byte[]](10, 0, 2, 2)
$gatewayMac = [byte[]](0x52, 0x54, 0x00, 0x00, 0x00, 0x02)

function Read-Exact {
    param(
        [System.Net.Sockets.NetworkStream]$Stream,
        [byte[]]$Buffer,
        [int]$Offset,
        [int]$Count
    )
    $read = 0
    while ($read -lt $Count) {
        $countRead = $Stream.Read($Buffer, $Offset + $read, $Count - $read)
        if ($countRead -le 0) { return $false }
        $read += $countRead
    }
    return $true
}

function Write-Frame {
    param(
        [System.Net.Sockets.NetworkStream]$Stream,
        [byte[]]$Frame
    )
    $length = $Frame.Length
    $prefix = [byte[]](
        [byte]($length -shr 24),
        [byte]($length -shr 16),
        [byte]($length -shr 8),
        [byte]$length
    )
    $Stream.Write($prefix, 0, $prefix.Length)
    $Stream.Write($Frame, 0, $Frame.Length)
    $Stream.Flush()
}

function Get-Checksum {
    param(
        [byte[]]$Bytes,
        [int]$Offset,
        [int]$Length
    )
    [uint32]$sum = 0
    $end = $Offset + $Length
    for ($index = $Offset; $index + 1 -lt $end; $index += 2) {
        $sum += ([uint32]$Bytes[$index] -shl 8) -bor [uint32]$Bytes[$index + 1]
    }
    if ($Offset + $Length -lt $end + 1 -and ($Length % 2) -ne 0) {
        $sum += [uint32]$Bytes[$end - 1] -shl 8
    }
    while ($sum -gt 0xffff) {
        $sum = ($sum -band 0xffff) + ($sum -shr 16)
    }
    return [uint16]((0xffff - $sum) -band 0xffff)
}

function New-ArpReply {
    param([byte[]]$Request)
    if ($Request.Length -lt 42) { return $null }
    if ($Request[20] -ne 0 -or $Request[21] -ne 1) { return $null }
    for ($index = 0; $index -lt 4; $index++) {
        if ($Request[38 + $index] -ne $gatewayIp[$index]) { return $null }
    }

    $reply = New-Object byte[] 60
    [Array]::Copy($Request, 6, $reply, 0, 6)
    [Array]::Copy($gatewayMac, 0, $reply, 6, 6)
    $reply[12] = 0x08
    $reply[13] = 0x06
    $reply[14] = 0x00
    $reply[15] = 0x01
    $reply[16] = 0x08
    $reply[17] = 0x00
    $reply[18] = 0x06
    $reply[19] = 0x04
    $reply[20] = 0x00
    $reply[21] = 0x02
    [Array]::Copy($gatewayMac, 0, $reply, 22, 6)
    [Array]::Copy($gatewayIp, 0, $reply, 28, 4)
    [Array]::Copy($Request, 6, $reply, 32, 6)
    [Array]::Copy($Request, 28, $reply, 38, 4)
    return [byte[]]$reply
}

function New-IcmpReply {
    param([byte[]]$Request)
    if ($Request.Length -lt 42) { return $null }
    $ipOffset = 14
    $ihl = ($Request[$ipOffset] -band 0x0f) * 4
    $ipLength = ([int]$Request[$ipOffset + 2] -shl 8) -bor $Request[$ipOffset + 3]
    $icmpOffset = $ipOffset + $ihl
    if ($Request[$ipOffset + 9] -ne 1 -or $ipLength -lt $ihl + 8 -or $Request[$icmpOffset] -ne 8) {
        return $null
    }
    $frameLength = [Math]::Max(60, $ipOffset + $ipLength)
    $reply = New-Object byte[] $frameLength
    [Array]::Copy($Request, 0, $reply, 0, [Math]::Min($Request.Length, $reply.Length))
    [Array]::Copy($Request, 0, $reply, 6, 6)
    [Array]::Copy($Request, 6, $reply, 0, 6)
    for ($index = 0; $index -lt 4; $index++) {
        $reply[$ipOffset + 12 + $index] = $Request[$ipOffset + 16 + $index]
        $reply[$ipOffset + 16 + $index] = $Request[$ipOffset + 12 + $index]
    }
    $reply[$icmpOffset] = 0
    $reply[$icmpOffset + 2] = 0
    $reply[$icmpOffset + 3] = 0
    $icmpChecksum = Get-Checksum $reply $icmpOffset ($ipLength - $ihl)
    $reply[$icmpOffset + 2] = [byte]($icmpChecksum -shr 8)
    $reply[$icmpOffset + 3] = [byte]($icmpChecksum -band 0xff)
    $reply[$ipOffset + 10] = 0
    $reply[$ipOffset + 11] = 0
    $ipChecksum = Get-Checksum $reply $ipOffset $ihl
    $reply[$ipOffset + 10] = [byte]($ipChecksum -shr 8)
    $reply[$ipOffset + 11] = [byte]($ipChecksum -band 0xff)
    return [byte[]]$reply
}

function Read-UInt32 {
    param([byte[]]$Bytes, [int]$Offset)
    return ([uint32]$Bytes[$Offset] -shl 24) -bor
        ([uint32]$Bytes[$Offset + 1] -shl 16) -bor
        ([uint32]$Bytes[$Offset + 2] -shl 8) -bor
        [uint32]$Bytes[$Offset + 3]
}

function Write-UInt16 {
    param([byte[]]$Bytes, [int]$Offset, [uint16]$Value)
    $Bytes[$Offset] = [byte](($Value -shr 8) -band 0xff)
    $Bytes[$Offset + 1] = [byte]($Value -band 0xff)
}

function Write-UInt32 {
    param([byte[]]$Bytes, [int]$Offset, [uint32]$Value)
    $Bytes[$Offset] = [byte](($Value -shr 24) -band 0xff)
    $Bytes[$Offset + 1] = [byte](($Value -shr 16) -band 0xff)
    $Bytes[$Offset + 2] = [byte](($Value -shr 8) -band 0xff)
    $Bytes[$Offset + 3] = [byte]($Value -band 0xff)
}

function New-TcpSynAck {
    param([byte[]]$Request)
    if ($Request.Length -lt 54) { return $null }
    $ipOffset = 14
    $ihl = ($Request[$ipOffset] -band 0x0f) * 4
    $ipLength = ([int]$Request[$ipOffset + 2] -shl 8) -bor $Request[$ipOffset + 3]
    $tcpOffset = $ipOffset + $ihl
    if ($Request[$ipOffset + 9] -ne 6 -or $ipLength -lt $ihl + 20 -or $Request.Length -lt $tcpOffset + 20) {
        return $null
    }
    $flags = $Request[$tcpOffset + 13]
    if (($flags -band 0x02) -eq 0 -or ($flags -band 0x10) -ne 0) { return $null }

    $reply = New-Object byte[] 60
    [Array]::Copy($Request, 6, $reply, 0, 6)
    [Array]::Copy($gatewayMac, 0, $reply, 6, 6)
    $reply[12] = 0x08
    $reply[13] = 0x00
    $reply[$ipOffset] = 0x45
    $reply[$ipOffset + 1] = 0
    Write-UInt16 $reply ($ipOffset + 2) 40
    Write-UInt16 $reply ($ipOffset + 4) 1
    Write-UInt16 $reply ($ipOffset + 6) 0
    $reply[$ipOffset + 8] = 64
    $reply[$ipOffset + 9] = 6
    $reply[$ipOffset + 10] = 0
    $reply[$ipOffset + 11] = 0
    [Array]::Copy($gatewayIp, 0, $reply, $ipOffset + 12, 4)
    [Array]::Copy($Request, $ipOffset + 12, $reply, $ipOffset + 16, 4)
    $ipChecksum = Get-Checksum $reply $ipOffset $ihl
    Write-UInt16 $reply ($ipOffset + 10) $ipChecksum

    [Array]::Copy($Request, $tcpOffset + 2, $reply, $tcpOffset, 2)
    [Array]::Copy($Request, $tcpOffset, $reply, $tcpOffset + 2, 2)
    Write-UInt32 $reply ($tcpOffset + 4) 0x01020304
    $guestSequence = Read-UInt32 $Request ($tcpOffset + 4)
    Write-UInt32 $reply ($tcpOffset + 8) (($guestSequence + 1) -band 0xffffffff)
    $reply[$tcpOffset + 12] = 0x50
    $reply[$tcpOffset + 13] = 0x12
    Write-UInt16 $reply ($tcpOffset + 14) 65535
    Write-UInt16 $reply ($tcpOffset + 16) 0
    Write-UInt16 $reply ($tcpOffset + 18) 0

    $pseudo = New-Object byte[] 32
    [Array]::Copy($reply, $ipOffset + 12, $pseudo, 0, 4)
    [Array]::Copy($reply, $ipOffset + 16, $pseudo, 4, 4)
    $pseudo[9] = 6
    Write-UInt16 $pseudo 10 20
    [Array]::Copy($reply, $tcpOffset, $pseudo, 12, 20)
    $tcpChecksum = Get-Checksum $pseudo 0 $pseudo.Length
    Write-UInt16 $reply ($tcpOffset + 16) $tcpChecksum
    return [byte[]]$reply
}

function New-TcpSyn {
    param([byte[]]$Request)
    if ($Request.Length -lt 54) { return $null }
    $ipOffset = 14
    $ihl = ($Request[$ipOffset] -band 0x0f) * 4
    $tcpOffset = $ipOffset + $ihl
    if ($Request[$ipOffset + 9] -ne 1 -or $Request.Length -lt $tcpOffset + 20) { return $null }

    $reply = New-Object byte[] 60
    [Array]::Copy($Request, 6, $reply, 0, 6)
    [Array]::Copy($gatewayMac, 0, $reply, 6, 6)
    $reply[12] = 0x08
    $reply[13] = 0x00
    $reply[$ipOffset] = 0x45
    Write-UInt16 $reply ($ipOffset + 2) 40
    Write-UInt16 $reply ($ipOffset + 4) 2
    Write-UInt16 $reply ($ipOffset + 6) 0
    $reply[$ipOffset + 8] = 64
    $reply[$ipOffset + 9] = 6
    [Array]::Copy($gatewayIp, 0, $reply, $ipOffset + 12, 4)
    [Array]::Copy($Request, $ipOffset + 12, $reply, $ipOffset + 16, 4)
    Write-UInt16 $reply ($ipOffset + 10) 0
    Write-UInt16 $reply ($ipOffset + 10) (Get-Checksum $reply $ipOffset $ihl)

    Write-UInt16 $reply $tcpOffset 45000
    Write-UInt16 $reply ($tcpOffset + 2) 8081
    Write-UInt32 $reply ($tcpOffset + 4) 0x0a0b0c0d
    Write-UInt32 $reply ($tcpOffset + 8) 0
    $reply[$tcpOffset + 12] = 0x50
    $reply[$tcpOffset + 13] = 0x02
    Write-UInt16 $reply ($tcpOffset + 14) 65535
    Write-UInt16 $reply ($tcpOffset + 16) 0
    Write-UInt16 $reply ($tcpOffset + 18) 0

    $pseudo = New-Object byte[] 32
    [Array]::Copy($reply, $ipOffset + 12, $pseudo, 0, 4)
    [Array]::Copy($reply, $ipOffset + 16, $pseudo, 4, 4)
    $pseudo[9] = 6
    Write-UInt16 $pseudo 10 20
    [Array]::Copy($reply, $tcpOffset, $pseudo, 12, 20)
    Write-UInt16 $reply ($tcpOffset + 16) (Get-Checksum $pseudo 0 $pseudo.Length)
    return [byte[]]$reply
}

function New-TcpAckData {
    param(
        [byte[]]$Request,
        [uint32]$Sequence,
        [uint32]$Acknowledgement,
        [byte[]]$Payload
    )
    if ($Request.Length -lt 54) { return $null }
    $ipOffset = 14
    $ihl = ($Request[$ipOffset] -band 0x0f) * 4
    $tcpOffset = $ipOffset + $ihl
    if ($Request[$ipOffset + 9] -ne 6 -or $Request.Length -lt $tcpOffset + 20) { return $null }
    $tcpLength = 20 + $Payload.Length
    $ipLength = $ihl + $tcpLength
    $frameLength = [Math]::Max(60, $ipOffset + $ipLength)
    $reply = New-Object byte[] $frameLength
    [Array]::Copy($Request, 6, $reply, 0, 6)
    [Array]::Copy($gatewayMac, 0, $reply, 6, 6)
    $reply[12] = 0x08
    $reply[13] = 0x00
    $reply[$ipOffset] = 0x45
    Write-UInt16 $reply ($ipOffset + 2) $ipLength
    Write-UInt16 $reply ($ipOffset + 4) 3
    Write-UInt16 $reply ($ipOffset + 6) 0
    $reply[$ipOffset + 8] = 64
    $reply[$ipOffset + 9] = 6
    [Array]::Copy($Request, $ipOffset + 16, $reply, $ipOffset + 12, 4)
    [Array]::Copy($Request, $ipOffset + 12, $reply, $ipOffset + 16, 4)
    Write-UInt16 $reply ($ipOffset + 10) 0
    Write-UInt16 $reply ($ipOffset + 10) (Get-Checksum $reply $ipOffset $ihl)

    [Array]::Copy($Request, $tcpOffset + 2, $reply, $tcpOffset, 2)
    [Array]::Copy($Request, $tcpOffset, $reply, $tcpOffset + 2, 2)
    Write-UInt32 $reply ($tcpOffset + 4) $Sequence
    Write-UInt32 $reply ($tcpOffset + 8) $Acknowledgement
    $reply[$tcpOffset + 12] = 0x50
    $reply[$tcpOffset + 13] = 0x10
    Write-UInt16 $reply ($tcpOffset + 14) 65535
    Write-UInt16 $reply ($tcpOffset + 16) 0
    Write-UInt16 $reply ($tcpOffset + 18) 0
    if ($Payload.Length -gt 0) {
        [Array]::Copy($Payload, 0, $reply, $tcpOffset + 20, $Payload.Length)
    }

    $pseudo = New-Object byte[] (12 + $tcpLength)
    [Array]::Copy($reply, $ipOffset + 12, $pseudo, 0, 4)
    [Array]::Copy($reply, $ipOffset + 16, $pseudo, 4, 4)
    $pseudo[9] = 6
    Write-UInt16 $pseudo 10 $tcpLength
    [Array]::Copy($reply, $tcpOffset, $pseudo, 12, $tcpLength)
    Write-UInt16 $reply ($tcpOffset + 16) (Get-Checksum $pseudo 0 $pseudo.Length)
    return [byte[]]$reply
}

function Handle-Frame {
    param([byte[]]$Frame)
    if ($Frame.Length -lt 14) { return $null }
    $ethertype = ([int]$Frame[12] -shl 8) -bor $Frame[13]
    if ($ethertype -eq 0x0806) { return New-ArpReply $Frame }
    if ($ethertype -eq 0x0800) {
        $ipOffset = 14
        $ihl = ($Frame[$ipOffset] -band 0x0f) * 4
        $tcpOffset = $ipOffset + $ihl
        if ($Frame[$ipOffset + 9] -ne 6 -or $Frame.Length -lt $tcpOffset + 20) { return $null }
        $flags = $Frame[$tcpOffset + 13]
        if (($flags -band 0x12) -eq 0x12) {
            $guestSequence = Read-UInt32 $Frame ($tcpOffset + 4)
            $hostSequence = Read-UInt32 $Frame ($tcpOffset + 8)
            return New-TcpAckData $Frame $hostSequence (($guestSequence + 1) -band 0xffffffff) ([byte[]]@())
        }
        if (($flags -band 0x02) -ne 0 -and ($flags -band 0x10) -eq 0) {
            return New-TcpSynAck $Frame
        }
        if (($flags -band 0x10) -ne 0) {
            $guestSequence = Read-UInt32 $Frame ($tcpOffset + 4)
            $hostSequence = Read-UInt32 $Frame ($tcpOffset + 8)
            $dataOffset = (($Frame[$tcpOffset + 12] -shr 4) -band 0x0f) * 4
            $ipLength = ([int]$Frame[$ipOffset + 2] -shl 8) -bor $Frame[$ipOffset + 3]
            $payloadLength = [Math]::Max(0, $ipLength - $ihl - $dataOffset)
            if (($flags -band 0x01) -ne 0) { $payloadLength++ }
            return New-TcpAckData $Frame $hostSequence (($guestSequence + $payloadLength) -band 0xffffffff) ([byte[]]@())
        }
    }
    return $null
}

$client = $null
while ($true) {
    try {
        $client = [System.Net.Sockets.TcpClient]::new()
        $client.Connect('127.0.0.1', $Port)
        break
    } catch {
        if ($client) { $client.Dispose() }
        Start-Sleep -Milliseconds 100
    }
}

try {
    $stream = $client.GetStream()
    while ($true) {
        $prefix = New-Object byte[] 4
        if (-not (Read-Exact $stream $prefix 0 4)) { break }
        $length = ([int]$prefix[0] -shl 24) -bor ([int]$prefix[1] -shl 16) -bor
            ([int]$prefix[2] -shl 8) -bor [int]$prefix[3]
        if ($length -lt 14 -or $length -gt 2048) { break }
        $frame = New-Object byte[] $length
        if (-not (Read-Exact $stream $frame 0 $length)) { break }
        $icmp = New-IcmpReply $frame
        if ($icmp) {
            Write-Frame $stream $icmp
            $syn = New-TcpSyn $frame
            if ($syn) { Write-Frame $stream $syn }
            continue
        }
        $reply = Handle-Frame $frame
        if ($reply) {
            Write-Frame $stream $reply
            $tcpOffset = 14 + (($frame[14] -band 0x0f) * 4)
            if ((([int]$frame[12] -shl 8) -bor $frame[13]) -eq 0x0800 -and
                $frame[23] -eq 6 -and $frame[$tcpOffset + 13] -eq 0x12) {
                $guestSequence = Read-UInt32 $frame ($tcpOffset + 4)
                $hostSequence = Read-UInt32 $frame ($tcpOffset + 8)
                $data = New-TcpAckData $frame $hostSequence (($guestSequence + 1) -band 0xffffffff) ([byte[]](0x4e))
                if ($data) {
                    Start-Sleep -Milliseconds 25
                    Write-Frame $stream $data
                }
            }
        }
    }
} finally {
    if ($client) { $client.Dispose() }
}
