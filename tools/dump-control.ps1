# Extracts the HX Stomp control-channel frames from a USB capture.
#
# Filters to the device's interrupt endpoints (EP 0x01 OUT / 0x81 IN) and prints each frame
# decoded into the header fields we currently understand, plus the raw hex. Reuse this on
# every capture so we can diff controlled actions byte-for-byte.
#
# Usage:  pwsh tools/dump-control.ps1 captures/NN-action.pcapng [-Address 8]
param(
    [Parameter(Mandatory = $true)][string]$Pcap,
    [int]$Address = 8
)
$ts = "C:\Program Files\Wireshark\tshark.exe"
if (-not (Test-Path $ts)) { throw "tshark not found at $ts" }

$filter = "usb.device_address==$Address && usb.transfer_type==0x03 && usb.capdata"
$rows = & $ts -r $Pcap -Y $filter -T fields `
    -e frame.number -e usb.endpoint_address -e usb.data_len -e usb.capdata 2>$null

"{0,5}  {1}  {2,-4}  {3,-5} {4,-5} {5,-5} {6,-5}  {7}" -f `
    "frame", "dir", "len", "idA", "idB", "seq", "f10", "payload(hex from offset 16)"
"-" * 100
foreach ($line in $rows) {
    $c = $line -split "`t"
    if ($c.Count -lt 4) { continue }
    $frame = $c[0]; $ep = $c[1]; $len = [int]$c[2]; $hex = $c[3]
    $dir = if ($ep -eq "0x81") { "IN " } else { "OUT" }
    # Header fields (hypotheses — see docs/protocol.md). Bytes are hex pairs in $hex.
    $b = for ($i = 0; $i -lt $hex.Length; $i += 2) { $hex.Substring($i, 2) }
    $idA = "$($b[4])$($b[5])"   # channel id #1 (dst on OUT / src on IN)
    $idB = "$($b[6])$($b[7])"   # channel id #2
    $seq = "$($b[8])$($b[9])"   # sequence counter (big-endian)
    $f10 = "$($b[10])$($b[11])" # message-class / sub-count (0010 steady, 0004/0008 on events)
    $payload = if ($len -gt 16) { ($b[16..($len - 1)] -join "") } else { "" }
    "{0,5}  {1}  {2,-4}  {3,-5} {4,-5} {5,-5} {6,-5}  {7}" -f $frame, $dir, $len, $idA, $idB, $seq, $f10, $payload
}
