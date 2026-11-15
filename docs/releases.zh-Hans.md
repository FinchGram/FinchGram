# 发布

[English](releases.md)

release 只由 GitHub Actions 在干净的机器上、从打了 tag 的 commit 构建（`.github/workflows/release.yml`），
发布在本仓库里：压缩好的 app、`SHA256SUMS` 和它的 Ed25519 签名。已安装的 app 从这里自动更新，
验证不过的一律不装。维护者用一条命令发起 release：

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1；也可以是 minor、major 或一个具体版本号
```

app 用 North Brookville LLC 的 Apple Developer ID 签名，开启 Hardened Runtime，并经 Apple 公证：下载的 app 打开时
Gatekeeper 不会拦，app 自己安装的更新也一样。

每个 release 都带 `SHA256SUMS` 和它的 Ed25519 签名。app 装更新之前会先校验这两样；你也可以自己校验，公钥在
[release-signing.pub](../release-signing.pub)。
