# Where researchers lose time

A survey done in November 2026 to decide what to build: issue trackers, forums (Biostars, the QIIME 2
forum, r/bioinformatics), papers, and the current state of tools. Evidence strength is noted, because
there is no good survey of how researchers split their time; the widely quoted "80% of time on data
wrangling" comes from an opinion piece, not data.

## 1. Manual steps worth a CLI

Ranked by how often they recur and how well they're documented.

| # | Manual step | Evidence | Already exists | Our answer |
|---|---|---|---|---|
| 1 | Building sample sheets and manifests from FASTQ folders | Moderate: each nf-core pipeline vendors its own `fastq_dir_to_samplesheet.py` (no subfolders); recurring QIIME 2 manifest errors | Per-pipeline scripts, fetchngs (public data only) | [`sheetgen`](../tools/sheetgen.md) |
| 2 | Downloading, versioning and pinning reference databases | Moderate to strong: Bakta's one-schema-per-release rule and MD5 failures; Kraken 2 version/taxonomy mismatches giving empty output; database choice changes results | Each tool's downloader, refgenie (genome-centric), Galaxy data managers | [`dblock`](../tools/dblock.md) |
| 3 | Installing tools, dependency conflicts | Strong (28% of 98 omics tools failed to install in 2 h, Mangul et al. 2019) | Bioconda, BioContainers, pixi | Mostly solved; single binaries sidestep it |
| 4 | Debugging pipeline runs on HPC/cloud | Strong (studies of 25,000+ nf-core issues) | Nextflow/Seqera tooling | Too platform-specific for a small tool |
| 5 | Merging AMR/virulence calls from several tools | Moderate (PHA4GE's hAMRonization exists for this) | hAMRonization, abricate `--summary` | Contribute upstream rather than duplicate |
| 6 | One QC row per isolate (reads, species, assembly, completeness, AMR) | Moderate, indirect: pipelines keep writing their own summary scripts | MultiQC (per tool, not per isolate) | Planned: `isotable` |
| 7 | Preparing NCBI/ENA submissions | Moderate (documentation, not complaints) | Webin-CLI, SeqSender | Planned: `subprep` |
| 8 | Fetching public reads with metadata | Strong, but well served | kingfisher, fastq-dl, NCBI datasets | Skip |
| 9 | GTDB ↔ NCBI taxonomy mapping | Moderate | GTDB mapping files, taxonkit | Maybe later |
| 10 | Annotating trees for iTOL | Weak to moderate; crowded | table2itol, itol.toolkit | Skip |

## 2. Viewers

Which data types would gain most from a fast, GPU-rendered native viewer (planned in Rust with
GPUI):

| Data | Main tools | Complaint | Gap |
|---|---|---|---|
| Multiple sequence alignments | Jalview, AliView (Java), SeaView | Silent out-of-memory, a few hundred rows on screen, stalls at 10^4–10^5 sequences | **Large**: no native viewer for 10^5–10^6 sequences; a WebGL viewer showed 10–100× gains |
| Read alignments (BAM/CRAM) | IGV (Java), igv.js, JBrowse 2 | Java heap tuning, deep coverage, long reads | Medium: IGV is trusted, must win on speed |
| Phylogenetic trees | FigTree, iTOL, Taxonium | Older tools manage thousands of tips | Small: Taxonium already handles millions |
| Assembly graphs | Bandage / BandageNG | Minutes to lay out, 12 GB RAM | Large but niche; layout is the hard part |
| Metagenomic overviews | Krona, anvi'o | Browser limits, O(n²) clustering | Medium |

First viewer: **multiple sequence alignments**, with a linked tree panel next.

## 3. AI agents for bioinformatics

The field is busy: academic agents (Biomni, BioMaster, AutoBA, CellAgent), commercial ones (Edison
Kosmos, Seqera AI and Nextflow's agent step, Latch, Benchling), and MCP servers (Galaxy's official
one, BioinfoMCP, many database wrappers).

Benchmarks show the limits. BixBench is close to saturated, but on BixBench3 (August 2026, study-scale
tasks) the best score was 0.48, falling to 0.10 on datasets over 100 GB, at about $43 a task.
Documented failure modes: carrying on silently after bad input, invented command-line flags, no
record that lets a run be replayed without the AI, and unsandboxed execution.

Gaps a small team can fill, and our order for them:

1. A typed MCP tool layer over standard microbial genomics tools (fastp, SPAdes, Bakta, CheckM2,
   GTDB-Tk, AMRFinderPlus) with checked flags and pinned versions. None exists for Bakta, CheckM2 or
   GTDB-Tk.
2. Recording every call as a replayable script (and RO-Crate), so results don't depend on the AI.
3. A cheap evaluation harness on small microbial datasets, including deliberately broken inputs.

An unexplored angle: nobody measures these agents on laptop-class hardware.

## Sources

Collected during the survey; the most load-bearing ones:

- nf-core sample sheet scripts: [rnaseq#900](https://github.com/nf-core/rnaseq/issues/900), [rnaseq#995](https://github.com/nf-core/rnaseq/issues/995)
- QIIME 2 manifest errors: [forum 21786](https://forum.qiime2.org/t/importing-data-manifest-file-not-recognized/21786), [q2-types#229](https://github.com/qiime2/q2-types/issues/229)
- Bakta database issues: [#417](https://github.com/oschwengers/bakta/issues/417), [#283](https://github.com/oschwengers/bakta/issues/283)
- Tool installability: [Mangul et al., PLoS Biol 2019](https://journals.plos.org/plosbiology/article?id=10.1371%2Fjournal.pbio.3000333)
- nf-core issue studies: [arXiv 2601.09612](https://arxiv.org/abs/2601.09612), [arXiv 2607.10839](https://arxiv.org/abs/2607.10839)
- hAMRonization: [GitHub](https://github.com/pha4ge/hAMRonization)
- MSA viewers: [Jalview FAQ](https://jalview.github.io/help/faq/), [AliView paper](https://academic.oup.com/bioinformatics/article/30/22/3276/2391211), [Alignment Viewer 2.0](https://github.com/sanderlab/AlignmentViewer2.0)
- Taxonium: [eLife](https://elifesciences.org/articles/82392)
- Agents and benchmarks: [Biomni](https://github.com/snap-stanford/Biomni), [BixBench](https://arxiv.org/abs/2503.00096), [BixBench3](https://arxiv.org/abs/2608.25286), [BioAgent Bench](https://arxiv.org/abs/2601.21800), [Galaxy MCP](https://github.com/galaxyproject/galaxy-mcp)
