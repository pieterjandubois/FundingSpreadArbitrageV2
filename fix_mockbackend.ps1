$file = "src/strategy/atomic_execution.rs"
$content = Get-Content $file -Raw

# Pattern to match backend_name method followed by closing brace
$pattern = '(fn backend_name\(&self\) -> &str \{\s+"(?:mock|MockBackend)"\s+\}\s*)\n(\s*\})'

# Replacement adds get_quantity_step before the closing brace
$replacement = '$1' + "`n`n            async fn get_quantity_step(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {`n                Ok(0.001)`n            }`n" + '$2'

$newContent = $content -replace $pattern, $replacement

Set-Content $file -Value $newContent -NoNewline
Write-Host "Fixed MockBackend implementations"
