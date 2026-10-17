param([string]$Path, [int]$Min = 6, [string]$Filter = '.')
# Extract ASCII and UTF-16LE strings from a binary, filtered by regex.
$b = [IO.File]::ReadAllBytes($Path)
$out = New-Object System.Collections.Generic.List[string]
$sb = New-Object System.Text.StringBuilder
foreach ($x in $b) {
    if ($x -ge 0x20 -and $x -lt 0x7f) { [void]$sb.Append([char]$x) }
    else { if ($sb.Length -ge $Min) { $out.Add("A " + $sb.ToString()) }; [void]$sb.Clear() }
}
$u = [Text.Encoding]::Unicode.GetString($b)
foreach ($m in [regex]::Matches($u, "[ -~ -ɏ]{$Min,}")) { $out.Add("U " + $m.Value) }
$out | Where-Object { $_ -match $Filter } | Select-Object -Unique
