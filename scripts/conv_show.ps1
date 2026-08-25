$ErrorActionPreference = 'Stop'
$f = "C:\Users\hanpengfei7\.accio\accounts\1779684550\conversations\dm\CID-24684550U1787671-AEA2AA-3346-1F64DC.message_1.jsonl"
$lines = Get-Content -LiteralPath $f -Encoding UTF8
$i = 0
foreach ($l in $lines) {
    try { $o = $l | ConvertFrom-Json } catch { $i++; continue }
    if ($o.name -eq 'sessions_spawn' -or $o.status -eq 'error' -or $o.name -eq 'list') {
        $content = [string]$o.content
        if ($content.Length -gt 700) { $content = $content.Substring(0,700) + " ...[trunc]" }
        Write-Host ("==== line {0} role={1} name={2} status={3} ====" -f $i,$o.role,$o.name,$o.status)
        Write-Host $content
        Write-Host ""
    }
    $i++
}