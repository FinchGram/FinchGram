# Releases

[中文](releases.zh-Hans.md)

Releases are built only by GitHub Actions, from a tagged commit on a clean runner
(`.github/workflows/release.yml`), and published in this repository: the zipped app,
`SHA256SUMS`, and its Ed25519 signature. Installed copies update themselves from there and install
nothing they cannot verify. A maintainer starts a release with one command:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; also minor, major, or an exact version
```

The app is signed with North Brookville LLC's Apple Developer ID, with the hardened runtime, and
notarized by Apple: a downloaded copy opens without a question from Gatekeeper, and updates
installed by the app itself do too.

Every release comes with `SHA256SUMS` and its Ed25519 signature. The app checks both before it
installs an update; so can you, with the public key in [release-signing.pub](../release-signing.pub).
