# Install

## Homebrew

```bash
brew tap holon-run/homebrew-tap
brew install uxc
```

## Install Script

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash -s -- -v v0.15.1
```

## Cargo

```bash
cargo install uxc
```

## From Source

```bash
git clone https://github.com/holon-run/uxc.git
cd uxc
cargo install --path .
```

Windows note: run UXC through WSL.
