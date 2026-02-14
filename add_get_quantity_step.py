#!/usr/bin/env python3
import re

file_path = "src/strategy/atomic_execution.rs"

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# Find all occurrences of backend_name method and add get_quantity_step after it
# Pattern: fn backend_name(&self) -> &str { "mock" or "MockBackend" } followed by closing brace of impl
pattern = r'(fn backend_name\(&self\) -> &str \{\s+(?:"mock"|"MockBackend")\s+\})\s*(\n\s*\})'

def replacement(match):
    return match.group(1) + '\n\n            async fn get_quantity_step(&self, _exchange: &str, _symbol: &str) -> Result<f64, Box<dyn Error + Send + Sync>> {\n                Ok(0.001)\n            }' + match.group(2)

new_content = re.sub(pattern, replacement, content)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(new_content)

print("Added get_quantity_step method to all MockBackend implementations")
