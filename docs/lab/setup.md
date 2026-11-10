# Running nf-core on a laptop

Notes from setting up a standard microbial genomics pipeline on an ordinary Linux laptop, to see
where it strains. Results are in [benchmarks](../benchmarks.md#nf-corebacass-baseline).

## What's needed

- **Java 17+** for Nextflow. We pinned Temurin 21 per project with [mise](https://mise.jdx.dev):
  `mise use java@temurin-21`.
- **Nextflow**: `curl -s https://get.nextflow.io | bash`, then move `nextflow` onto your `PATH`.
  Setting `NXF_HOME=$PWD/.nextflow-home` keeps its cache inside the project.
- **Docker** (or Podman/Apptainer) for the tool containers; your user in the `docker` group.

## Keep the pipeline from taking the whole machine

A `local.config` passed with `-c local.config`:

```groovy
process.resourceLimits = [cpus: 8, memory: 12.GB, time: 6.h]
docker.enabled = true
```

Adjust to leave room for your desktop: on an 8 GB laptop, `memory: 6.GB` and `cpus: 4`.

## Run

```sh
nextflow run nf-core/bacass -profile test,docker -c local.config --outdir results/bacass-test
```

Use a pipeline's `test` profile with `--input your_sheet.tsv` to try your own sample sheet quickly;
it keeps the profile's settings that skip large databases. Add `-stub-run` to check inputs and
wiring without computing anything.

## Where it strains

- **Disk, not RAM or CPU.** About 16 GB of container images for one pipeline. Check
  `docker system df` and prune images you no longer need.
- **Reference databases.** Full Kraken 2 (~80 GB) and GTDB-Tk (~100 GB on disk, ~140 GB RAM) do not
  fit a typical laptop. Use the capped Kraken 2 indexes and Bakta's light database;
  [`dblock plan`](../tools/dblock.md) tells you what fits before downloading.
- **FastQC memory.** 2–3 GB per file inside the pipeline; [`leanqc`](../tools/leanqc.md) does the
  same job in tens of MB.

## Gotchas we hit

- Pipeline parameters given as bare flags (`--skip_kraken2`) arrived as the string `"true"` and
  failed schema validation with Nextflow 26.10. Use `-params-file params.json` or the test profile.
- With only a sample sheet swapped into the test profile, bacass still asks for Kraken 2 and
  Kmerfinder databases unless the profile's `skip_kraken2`/`skip_kmerfinder` settings are kept.
