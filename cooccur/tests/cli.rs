//! Runs the cooccur binary on a table that coverm genome 0.8.0 wrote (`-m relative_abundance count
//! mean --min-covered-fraction 0`, four 30 kb genomes, six samples of simulated reads in which
//! bin.2 got twice bin.1's share).

use std::process::Command;

const COVERM: &str = "Genome\tS1.fq Relative Abundance (%)\tS1.fq Read Count\tS1.fq Mean\tS2.fq Relative Abundance (%)\tS2.fq Read Count\tS2.fq Mean\tS3.fq Relative Abundance (%)\tS3.fq Read Count\tS3.fq Mean\tS4.fq Relative Abundance (%)\tS4.fq Read Count\tS4.fq Mean\tS5.fq Relative Abundance (%)\tS5.fq Read Count\tS5.fq Mean\tS6.fq Relative Abundance (%)\tS6.fq Read Count\tS6.fq Mean
unmapped\t0\tNA\tNA\t0\tNA\tNA\t0\tNA\tNA\t0\tNA\tNA\t0\tNA\tNA\t0\tNA\tNA
bin.1\t6.5468707\t262\t1.3145393\t12.296147\t492\t2.4685762\t19.686451\t787\t3.9530988\t20.827875\t833\t4.1805696\t23.530533\t941\t4.724087\t26.672918\t1067\t5.356717
bin.2\t13.592111\t544\t2.7291458\t25.577995\t1023\t5.1350417\t38.048573\t1522\t7.640268\t41.00132\t1640\t8.229782\t48.52048\t1941\t9.741173\t55.401623\t2216\t11.126298
bin.3\t41.31458\t1652\t8.295511\t29.689507\t1188\t5.9604692\t28.141617\t1126\t5.6509213\t15.052684\t602\t3.0213735\t9.811758\t392\t1.9698492\t4.297913\t172\t0.8631491
bin.4\t38.546432\t1542\t7.7396984\t32.43635\t1297\t6.511926\t14.123361\t565\t2.8360133\t23.118122\t925\t4.640268\t18.137236\t726\t3.6413066\t13.627545\t545\t2.7368174
";

const TAXONOMY: &str = "user_genome\tclassification\tfastani_reference
bin.1.fa\td__Bacteria;p__Patescibacteriota;c__Paceibacteria\tN/A
bin.2.fa\td__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria\tN/A
bin.3.fa\td__Bacteria;p__Bacillota;c__Bacilli\tN/A
";

fn cooccur(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cooccur")).args(args).output().expect("run cooccur")
}

#[test]
fn coverm_table_edges_and_summary() {
    let d = tempfile::tempdir().unwrap();
    let table = d.path().join("coverm.tsv");
    let tax = d.path().join("gtdbtk.bac120.summary.tsv");
    std::fs::write(&table, COVERM).unwrap();
    std::fs::write(&tax, TAXONOMY).unwrap();
    let t = table.to_str().unwrap();

    // three measures in one table: cooccur asks which
    let o = cooccur(&[t]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("pick one with --measure"));

    let edges = d.path().join("edges.tsv");
    let summary = d.path().join("summary.json");
    let o = cooccur(&[t, "-m", "count", "-t", tax.to_str().unwrap(), "-o", edges.to_str().unwrap(), "--summary", summary.to_str().unwrap(), "--threads", "2"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let e = std::fs::read_to_string(&edges).unwrap();
    let mut lines = e.lines();
    assert_eq!(lines.next().unwrap(), "genome_a\tgenome_b\trho\tp\tq\tpermutations\tn_both\tlabel_a\tlabel_b\tphylum_a\tphylum_b");
    let top: Vec<&str> = lines.next().unwrap().split('\t').collect();
    assert_eq!(&top[..2], ["bin.1", "bin.2"]);
    assert!(top[2].parse::<f64>().unwrap() > 0.99);
    assert_eq!(&top[6..], ["6", "CPR", "non-CPR", "p__Patescibacteriota", "p__Pseudomonadota"]);

    let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&summary).unwrap()).unwrap();
    assert_eq!(s["measure"], "Read Count");
    assert_eq!(s["samples"], 6);
    assert_eq!(s["labels"]["unclassified"], 1); // bin.4 is not in the taxonomy
    assert_eq!(s["cpr_pairs_passing"][0]["partner"], "bin.2");

    // same seed, other thread count: same edges
    let edges1 = d.path().join("edges1.tsv");
    let o = cooccur(&[t, "-m", "count", "-t", tax.to_str().unwrap(), "-o", edges1.to_str().unwrap(), "--threads", "1"]);
    assert!(o.status.success());
    assert_eq!(e, std::fs::read_to_string(&edges1).unwrap());

    // relative abundance and depth give the same rho to four decimals (no zeros here)
    let rho = |m: &str| -> String {
        let o = cooccur(&[t, "-m", m, "--all-pairs"]);
        let out = String::from_utf8(o.stdout).unwrap();
        let row = out.lines().find(|l| l.starts_with("bin.1\tbin.2\t")).unwrap().split('\t').nth(2).unwrap().to_string();
        row[..6].to_string()
    };
    assert_eq!(rho("relative_abundance"), rho("mean"));
}
