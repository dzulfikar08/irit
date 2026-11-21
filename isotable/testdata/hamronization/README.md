hAMRonization 1.4.1 outputs, written by `hamronize` itself (pip install hAMRonization==1.4.1) from
cut-down copies of the tool reports in hAMRonization's own test data
(https://github.com/pha4ge/hAMRonization, test/data/raw_outputs, at tag v1.4.1):

- `hamronized_abricate.tsv`: `hamronize abricate` on the first three hits of abricate/report.tsv
  (`--analysis_software_version 1.0.1 --reference_database_version 2024-01`).
- `hamronization_combined_report.tsv`: `hamronize summarize -t tsv` of that file and three more:
  `hamronize amrfinderplus` on the first three rows of amrfinderplus/report_point_v4.2.tsv
  (`--input_file_name assembly/GCF_010120755.1_ASM1012075v1_genomic.fna`) and on the blaTEM-156,
  sul2 and pmrB rows of amrfinderplus/report_protein.tsv (`--input_file_name
  assembly/SAMPLE_B.fasta`), both `--analysis_software_version 4.2.7
  --reference_database_version 2025-07-16.1`; and `hamronize rgi` on the NDM-1 and nalC rows of
  rgi/rgi.txt (`--input_file_name assembly/GCF_010120755.1_ASM1012075v1_genomic.fna
  --analysis_software_version 6.0.3 --reference_database_version 3.3.0`).
- `GCF_010120755.1_ASM1012075v1_genomic.amrfinder.tsv`: the same three AMRFinderPlus 4.2 rows
  (amrfinderplus/report_point_v4.2.tsv, header and first three rows) as a native AMRFinderPlus
  file for that genome.

The hits are hAMRonization's test data, not one real genome's results.
