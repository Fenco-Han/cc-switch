$ErrorActionPreference = 'Stop'
$f = "C:\Users\hanpengfei7\.accio\accounts\1779684550\conversations\dm\CID-24684550U1787671-AEA2AA-3346-1F64DC.message_1.jsonl"
$lines = Get-Content -LiteralPath $f -Encoding UTF8
$i = 0
foreach ($l in $lines) {
    try { $o = $l | ConvertFrom-Json } catch { $i++; continue }
    $role = $o.role; $name = $o.name; $status = $o.status
    $content = [string]$o.content
    $len = if($content){$content.Length}else{0}
    Write-Host ("[{0}] role={1} name={2} status={3} len={4}" -f $i,$role,$name,$status,$len)
    $i++
}