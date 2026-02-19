---
description: Always run manual testing after implementing each milestone or feature
---

## Manual Testing Checklist

After completing any milestone or feature implementation, you MUST run the following manual verification steps before marking it as complete:

### 1. Build Verification
// turbo
```
cargo build --workspace 2>&1 | tail -5
```

### 2. Unit Test Suite
// turbo
```
cargo test --workspace 2>&1 | grep -E '(test result|FAILED|error\[)' | head -20
```

### 3. Clippy Lint Check
// turbo
```
cargo clippy --workspace 2>&1 | grep -E '(warning|error)' | head -20
```

### 4. Binary Smoke Test
Run the compiled binary with `--help` to verify it still launches:
// turbo
```
cargo run -p rustcode -- --help 2>&1 | head -10
```

### 5. Feature-Specific Manual Tests
For each new module or changed module, manually verify:
- **New modules**: Confirm the module is importable and its public API works (run a targeted test like `cargo test -p <crate> <test_name>`)
- **Config/path changes**: Verify paths resolve correctly (e.g. check `~/.config/rustcode/` not `~/.config/opencode/`)
- **Auth changes**: Verify token expiry logic with edge cases
- **Plugin changes**: Verify plugin registration and trait implementation
- **LLM changes**: If possible, do a live LLM call with a real API key

### 6. Integration Test Sanity
// turbo
```
cargo test --workspace --test '*' 2>&1 | grep -E '(test result|FAILED)' | head -10
```

### 7. Document Results
Add a "Manual Verification" section to the walkthrough artifact with:
- Commands run and their output
- Any issues found and how they were resolved
- Confirmation of each feature working end-to-end
