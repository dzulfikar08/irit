# How this started

irit began in November 2026 as a side project between a software engineer (industrial software,
Rust, IoT) and a microbiologist heading into graduate work on environmental microbial ecology and
genomics. Neither wanted to start a biotech company; the aim was to build something useful and keep
current with computational biology.

## Ideas considered

Six directions came up and were compared on research demand, novelty, feasibility and how hard the
results are to validate:

| Idea | Verdict |
|---|---|
| AI workflow agent for bioinformatics pipelines | Easy to build, crowded (Biomni, BioMaster, Seqera AI, Galaxy MCP). Kept as a later layer on top of the tools, not the product. |
| Sequence analysis workbench | Mature free alternatives. Dropped. |
| Molecular docking / MD workbench | Compute-heavy and hard to validate. Dropped. |
| Multimodal microbial observatory (sensors plus periodic sequencing) | Strong fit for IoT skills, but needs field partners and years of data. Long-term. |
| Microbial interaction simulator ("bacteria battles") | Kept as a small outreach demo built on published two-species models, not a research product. |
| AI virtual cell (inspired by GenBio's AIDO Cell) | Multi-year research needing perturbation data that doesn't exist for most microbes. Out of scope. |

## What changed the plan

1. **Research over opinion.** Rather than build what one person found annoying, we surveyed issue
   trackers, forums and papers for where researchers lose time ([landscape](landscape.md)). The
   strongest signals were sample sheets, reference databases and QC aggregation: manual steps, not
   missing science.
2. **The laptop constraint.** The microbiologist works on a modest laptop, and so do many students.
   Running a standard pipeline locally showed the real bottlenecks were disk (16 GB of containers)
   and memory-hungry tools (FastQC, GTDB-Tk), not CPU. That became the organising principle: tools
   that work on the computer people actually have.
3. **Measured claims only.** Each tool is checked against the established tool it replaces or feeds
   before any number is published here.

## On AI "virtual cells"

GenBio AI's AIDO Cell (previewed August 2026) simulates two human cell lines (K562, HepG2) as a
persistent state you can perturb step by step. Its benchmark is self-reported and access is limited
to collaborators. The idea of a "virtual microbe" is appealing for microbial ecology, but such models
learn from large perturbation experiments, which can't be run on organisms that can't be cultured,
including much of environmental microbial diversity. We treat it as a direction to watch, not build.
