# Contributing to Codanna

See our [Contributing Guide](./contributing/README.md) for:

- Development Setup
- Code Guidelines - Mandatory Rust development principles  
- Testing - Local CI/CD scripts to verify changes
- Language Support - How to add new language parsers
- Pull Requests - How to submit contributions

## Quick Start

```bash
# Fork and clone
git clone https://github.com/bpstr/codanna.git
cd codanna

# Build
cargo build --release --all-features

# Test your changes
./contributing/scripts/quick-check.sh

# Auto-fix issues
./contributing/scripts/auto-fix.sh

# Full test before PR
./contributing/scripts/full-test.sh
```

## Key Resources

- [Development Guidelines](./contributing/development/guidelines.md) - Read before coding
- [Adding Language Support](./contributing/development/language-support.md) - Complete guide with checklist
- [Local CI/CD Scripts](./contributing/scripts/) - Test locally before pushing

## Getting Help

- Issues: Check existing [issues](https://github.com/bpstr/codanna/issues) or create new ones
- Discussions: Use GitHub Discussions for questions
- Before implementing: Create an issue first to discuss proposed changes