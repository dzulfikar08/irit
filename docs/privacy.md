# Privacy

irit's tools, msascope and fieldsheet run on your own computer or phone. None of them collects personal data, sends
analytics or usage statistics, or shows ads.

## msascope

msascope, including its Mac App Store edition, makes no network connections. It reads the files you
open and writes only the files you export. It keeps its settings (theme, panel sizes, recent files,
the reference row chosen in each file) in a settings file on your computer; deleting that file
removes them. On macOS that file is `~/Library/Application Support/msascope/settings.toml`, or
inside the app's container for the Mac App Store edition.

Help ▸ msascope Help and Acknowledgements open web pages or a local file in your browser; those
pages are on GitHub, whose own privacy policy applies to them.

## The command-line tools

- `dblock fetch` downloads the databases you add from the URLs in its catalog: the publishers,
  Zenodo, Amazon S3, NCBI, or irit's own data mirror on GitHub. `gtdbprep ani` downloads reference
  sketches only when you give it a `--source` (GTDB's server with `--source gtdb`, or a URL of
  your choosing). Those servers see your IP address, as with any download.
- `sheetgen verify --ena` asks ENA for the published MD5s and sizes of the run accessions it finds
  in file names. Without `--ena` it makes no connection.
- `irit-mcp` runs on your computer and talks to the AI client that started it. It makes network
  connections only through the tools above (`dblock_fetch` needs `confirm: true`; `gtdbprep_ani`
  needs a `source`; `reads_verify` needs `ena`) or the third-party programs it runs (Medaka, for
  example, downloads a named model the first time it is used). What the AI client sends to its AI
  provider is governed by the client and the provider.
- `scripts/install.sh` downloads irit from GitHub.

## fieldsheet

fieldsheet runs in your phone's browser. It is loaded from GitHub Pages (GitHub sees your IP
address, as for any web page) and then works offline. Samples, GPS positions (exact, coarse or not
recorded, set per project) and photos stay in the browser's storage on that device until you export
a CSV or a JSON backup. It makes no other network connections and sends nothing to us.

## Contact

Questions: open an issue at https://github.com/dzulfikar08/irit/issues.
