This is an early pre-release. It runs fully on your computer, and your audio never leaves it.

## Download

| Your computer | File |
|---|---|
| Mac with Apple Silicon (M1 or later) | `.dmg` ending in `aarch64` |
| Mac with an Intel chip | `.dmg` ending in `x64` |
| Windows 10 or 11 (64-bit) | `.exe` ending in `x64-setup` |
| Linux | `.AppImage`, `.deb` or `.rpm` |

## First launch

These builds are not signed by Apple or Microsoft, so your computer warns you the first time. The source code is public, and `SHA256SUMS` lists the checksum of every file.

**Mac**

1. Open the `.dmg` and drag SynthSpeech to Applications.
2. Open SynthSpeech and close the warning.
3. Go to **System Settings > Privacy & Security**, scroll down, and click **Open Anyway**.

If macOS says the app is damaged, run this in Terminal, then open it again:

```
xattr -dr com.apple.quarantine "/Applications/SynthSpeech.app"
```

**Windows**

1. Run the installer.
2. On "Windows protected your PC", click **More info**, then **Run anyway**.

Smart App Control blocks unsigned apps with no way to allow one. If it is turned on, SynthSpeech will not install.

## Known limits

- Windows is much slower than macOS and Linux right now.
- Windows and Linux run on the CPU, which needs AVX2 (most CPUs from 2013 on).
- There is no automatic update yet. Download each new version from this page.
