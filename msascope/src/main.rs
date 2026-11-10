use std::path::PathBuf;

use gpui_kit::*;
use msascope::Loaded;
use msascope::alphabet::SeqType;

mod gui;
use gui::app::{MsaScope, peak_rss_mb};
use gui::theme::{Settings, is_dark};

const USAGE: &str = "usage: msascope [FILE] [--type dna|rna|protein] [--stats] [--bench]
  FILE     aligned FASTA (.gz ok), Clustal or Stockholm
  --type   residue type, instead of guessing it from the residues (also for files opened later)
  --stats  load, print sizes and timings, and exit without opening a window
  --bench  open the window, pan through four zoom levels and print frame timings";

fn main() -> anyhow::Result<()> {
    let mut path = None;
    let (mut stats, mut bench) = (false, false);
    let mut ty: Option<SeqType> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let parse_ty = |v: Option<String>| -> anyhow::Result<SeqType> {
            let v = v.ok_or_else(|| anyhow::anyhow!("--type needs a value\n{USAGE}"))?;
            v.parse().map_err(|e| anyhow::anyhow!("{e}\n{USAGE}"))
        };
        match a.as_str() {
            "--type" => ty = Some(parse_ty(args.next())?),
            _ if a.starts_with("--type=") => ty = Some(parse_ty(Some(a["--type=".len()..].to_string()))?),
            "--stats" => stats = true,
            "--bench" => bench = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => path = Some(PathBuf::from(a)),
        }
    }
    if stats {
        let Some(p) = path else { anyhow::bail!("--stats needs a file\n{USAGE}") };
        let l = Loaded::open_as(&p, ty)?;
        println!("{}", l.summary());
        if let Some(rss) = peak_rss_mb() {
            println!("peak RSS {rss} MiB");
        }
        return Ok(());
    }
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let settings = Settings::load();
        gui::tokens::apply_palette(settings.theme.palette(is_dark(cx.window_appearance())), cx);
        gui::fonts::apply(None, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some("msascope".into()), ..Default::default() }),
            app_id: Some("msascope".into()),
            window_min_size: Some(size(px(640.), px(420.))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| MsaScope::new(path, ty, bench, window, cx)))
            .expect("failed to open window");
    });
    Ok(())
}
