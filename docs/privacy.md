# Privacy

irit's tools and msascope run on your own computer. None of them collects personal data, sends
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

- `dblock` downloads the reference databases you ask for from their publishers, and `gtdbprep ani`
  fetches GTDB reference sketches from GTDB's server. Those servers see your IP address, as with
  any download.
- `irit-mcp` runs on your computer and talks only to the AI client that started it. What that
  client sends to its AI provider is governed by the client and the provider.
- `scripts/install.sh` downloads irit from GitHub.

## Contact

Questions: open an issue at https://github.com/dzulfikar08/irit/issues.
