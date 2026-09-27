# winget manifests

Templates for the two winget packages; `scripts/release/winget.mjs` fills them from `product.json` and the
built release files (version, download URL, SHA-256) – the release pipeline runs it and attaches the
result as `winget-manifests.zip`.

| Package | Identifier | Type |
|---|---|---|
| Evaluation app | `<Publisher>.<Name>` | NSIS installer, per user, needs the WebView2 runtime (dependency `Microsoft.EdgeWebView2Runtime`) |
| Collector | `<Publisher>.<Name>.Collector` | portable program, command = collector file name |

Identifiers are built from `product.json` (`publisher`, `name`) with everything but letters and digits removed.

Submitting a release (after the GitHub release is published, so the download URLs work):

1. Download `winget-manifests.zip` from the release and unpack it.
2. `winget validate --manifest <folder>` for each package folder, then `winget install --manifest <folder>`
   in a Windows Sandbox as a smoke test.
3. Fork `microsoft/winget-pkgs`, copy the folders to `manifests/<first letter>/<Publisher>/<Name>/…`
   (the zip already has this layout) and open a pull request.
