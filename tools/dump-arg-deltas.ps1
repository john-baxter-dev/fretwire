param([string]$Pcap, [int]$Address = 9)
$ts = "C:\Program Files\Wireshark\tshark.exe"
$filter = "usb.device_address==$Address && usb.transfer_type==0x03 && usb.capdata"
$rows = & $ts -r $Pcap -Y $filter -T fields -e frame.number -e usb.endpoint_address -e usb.capdata 2>$null

# per (channelpair, direction) track previous arg to compute delta
$prev = @{}
foreach ($line in $rows) {
    $c = $line -split "`t"; if ($c.Count -lt 3) { continue }
    $frame=$c[0]; $ep=$c[1]; $hex=($c[2] -replace ':','');
    $b = for ($i=0; $i -lt $hex.Length; $i+=2){ $hex.Substring($i,2) }
    $n=$b.Count; if($n -lt 16){ continue }
    $dir = if($ep -eq "0x81"){"IN"}else{"OUT"}
    $src=$b[5]+$b[4]; $dst=$b[7]+$b[6]; $cmd=$b[11]
    $arg=[Convert]::ToInt64($b[15]+$b[14]+$b[13]+$b[12],16)
    $bodylen = $n - 16
    $key="$src/$dst/$dir"
    $delta = if($prev.ContainsKey($key)){ $arg - $prev[$key] } else { 0 }
    $prev[$key]=$arg
    "{0,6} {1,-3} {2}->{3} cmd={4} arg=0x{5:x6} d=+{6,-4} bodylen={7}" -f [int]$frame,$dir,$src,$dst,$cmd,$arg,$delta,$bodylen
}
