$file = "src/strategy/testnet_backend.rs"
$content = Get-Content $file -Raw

# Remove all "binance" => { ... } blocks in match statements
# Pattern 1: Simple binance match arms (most common)
$pattern1 = '(?s)\s+"binance"\s*=>\s*\{\s*if let Some\(client\)\s*=\s*&self\.binance\s*\{[^}]*\}\s*else\s*\{[^}]*\}\s*\}'
$content = $content -replace $pattern1, ''

# Pattern 2: Binance balance checking in get_all_balances
$pattern2 = '(?s)\s+if let Some\(client\)\s*=\s*&self\.binance\s*\{[^}]*match client\.get_balance\(\)\.await\s*\{[^}]*Ok\(balance\)[^}]*balances\.insert\("binance"[^}]*\}[^}]*Err\([^}]*\}[^}]*\}\s*\}'
$content = $content -replace $pattern2, ''

# Save the cleaned content
$content | Set-Content $file -NoNewline

Write-Host "Binance references removed successfully"
