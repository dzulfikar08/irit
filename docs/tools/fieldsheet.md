# fieldsheet: sample worksheet for the field

`fieldsheet` is a small web app for writing down sample metadata where the samples are taken: at a
water treatment site, by a river, at a lab bench. It runs in the phone's browser, works without internet once
opened, and exports a CSV with `sample` as the first column, so the same sample IDs carry through to
[`sheetgen`](sheetgen.md) and [`isotable`](isotable.md).

Open it at <https://dzulfikar08.github.io/irit/fieldsheet/>.

It is plain HTML, CSS and JavaScript (no framework, nothing loaded from other sites), 56 KB in all
(21 KB gzipped). The source is in [`site/public/fieldsheet/`](../../site/public/fieldsheet/).

## Install on a phone

1. Open the link above once while you have internet.
2. In the browser menu choose **Add to Home screen** (or **Install app**). On iPhone: Share,
   then **Add to Home Screen**.
3. Start it from the home-screen icon. From then on it opens offline: the app's files are kept by a
   service worker.

The interface is in English or Bahasa Indonesia; the **ID** / **EN** button in the top corner
switches.

## Use

- **Projects.** One project is one list of samples (a survey, a sampling trip). Each project uses a
  template:
  - *Water sample (metagenome)*: sample ID, site code, sampling point, date and time (filled
    in automatically), GPS, volume filtered (L), filter pore size(s) (µm), temperature, pH,
    conductivity, TDS, turbidity, free chlorine, notes and a photo.
  - *Isolate*: sample ID, source, species (guess), date, notes.

  **Project settings** adds custom fields (text, number with a range, date, or a choice list).
- **Add sample** opens a form prefilled with the last sample's values (date and time set to now,
  sample ID and photo left empty), so a series at one site is quick. **Save and add next** saves and
  opens the next one. Opening a sample and choosing **Duplicate (replicate)** copies it for a
  replicate. **Delete** asks first.
- **Sample IDs** are generated from the project's pattern when the ID is left empty. The water
  template's pattern is `{site}-{point}-{rep}`: site `D01`, point `S2` and the first free replicate
  letter give `D01-S2-A`, the next `D01-S2-B`. Tokens: any field key in braces (its value), `{rep}`
  (A, B, C…), `{n}`, `{nn}`, `{nnn}` (row number), `{date}` (YYYYMMDD). The isolate pattern is
  `ISO-{nnn}`.
- **Checks.** Required fields must be filled and numbers must be in range before a sample saves
  (pH 0–14, temperature −5–60 °C, free chlorine 0–20 mg/L, volume 0–1000 L, and so on). A comma is
  accepted as the decimal mark (`7,2`). A sample ID used twice, or one with characters other than
  letters, digits, `.`, `_` and `-` (sheetgen turns those into `_`), is flagged on the form and in
  the list, and a CSV export with duplicate IDs asks before going ahead.
- **Unsaved forms** are kept as a draft, so a reload or the phone's back button doesn't lose them.
  The **←** button in the app discards the draft.

Sampling point codes (the `point` column):

| Code | Point |
|---|---|
| `S1` | Source, before treatment |
| `S2` | After filter |
| `S3` | Dispensed (filling tap) |
| `S4` | Container, day 0 |
| `S5` | Container, stored |
| `BF` | Blank: field |
| `BE` | Blank: extraction |
| `BN` | Blank: no-template (PCR/library) |

## Where the data is

Everything is stored in the browser's IndexedDB on that device. Nothing is sent anywhere; the app
has no server. This also means:

- Clearing the browser's site data, or uninstalling the browser, deletes it. The app asks the
  browser for persistent storage, and the start screen says whether it was granted; without it the
  browser may clear the data when the phone is short of space.
- Data on the phone is not on the laptop until you move it: **Back up** writes a JSON file, and
  **Import a backup** on the other device reads it. Importing merges by record: new samples are
  added, a sample changed more recently replaces the older copy, the rest is left alone, so importing
  the same file twice changes nothing.
- Photos are shrunk to at most 1600 px and stored with the sample. They are never in the CSV, and in
  a JSON backup only when **Include photos in backups** is ticked (this makes the file much larger).
- GPS is optional. In **Project settings**, *Location precision* is *Exact*, *Coarse* (rounded to
  0.01°, about 1 km, recorded with an accuracy of at least 1100 m) or *Don't record*. Use *Coarse*
  or *Don't record* where a location identifies a person or a household.

See also [Privacy](../privacy.md).

## CSV to sheetgen and isotable

**Export CSV** (or **Share CSV** on phones that support sharing files) writes UTF-8 CSV with a
header row, one row per sample, `sample` first:

```
sample,site,point,collected,gps_lat,gps_lon,gps_accuracy_m,volume_l,filter_pore_um,temp_c,ph,...,notes
D01-S2-A,D01,S2,2026-11-14T17:27,-6.914744,107.60981,12,20,0.22;0.45,,7.2,...,"turbid, ""brownish"" / second line"
```

Numbers are written with a decimal point, several pore sizes are separated by `;`, `collected` is
the phone's local time, and line breaks inside notes become ` / ` (isotable reads sheets line by
line, so a multi-line cell would become an extra sample).

- **Sequencing.** Use these IDs as the sample names on the sequencing order. The provider's FASTQ
  files are then named after them, and [`sheetgen`](sheetgen.md) gives the same names in its sample
  sheet. sheetgen does not read the fieldsheet CSV itself: it builds its sheet from the FASTQ files.
- **Results.** [`isotable`](isotable.md) takes the CSV as `--sheet`: it reads the `sample` column
  and joins each tool's output to those names. Tested: with a fieldsheet export of three samples
  and one fastp report named `D01-S2-A.fastp.json`, `isotable results/ --sheet fieldsheet.csv`
  listed all three samples and put the fastp numbers on `D01-S2-A`. Samples with no output yet are
  listed as `no tool output found`, which shows what is still missing. isotable does not copy the
  metadata columns (pH, site, ...) into its table; join them on `sample` in a spreadsheet or script.

## Tested

In headless Chromium (Playwright, phone-sized 360×740 window, then a 1280×800 window in dark mode as
the "laptop"): creating a project; the empty form refused; auto IDs `D01-S2-A`, `D01-S2-B`,
`D01-S3-A`; pH 15 refused, `7,2` accepted; exact and coarse GPS; a photo; quick-add prefill;
duplicate for a replicate; a duplicate ID flagged; delete; data still there after reload; CSV
export; JSON backup with the photo, imported into a fresh browser profile (photo restored) and
re-imported (3 unchanged); the language switch; a custom choice field; and opening the app with the
network off (served by the service worker). 22 checks, all passed, no script errors.

## Known gaps

- Not yet tried on a real Android phone; *Add to Home screen*, the camera and GPS were tested only
  through the browser's emulation.
- Photo size after shrinking was not measured on real camera photos.
- No sync between devices other than backup files, by design.
- `collected` has no time-zone offset.
